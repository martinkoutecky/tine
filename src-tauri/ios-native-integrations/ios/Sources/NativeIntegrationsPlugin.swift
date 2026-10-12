import CoreSpotlight
import Foundation
import ObjectiveC
import SwiftRs
import Tauri
import UIKit
import WebKit

/// The App Group the app and its Share Extension share (ADR 0073). It must
/// match `src-tauri/Tine.ios.entitlements` and the extension's entitlements.
let tineAppGroup = "group.page.tine.Tine"
/// Darwin notification a producer posts after publishing an inbox item.
/// Must match `ios-app/Shared/ShareInbox.swift`.
let tineInboxChangedNotification = "page.tine.Tine.share-inbox-changed"
/// Core Spotlight domain of Tine's page items.
let tineSpotlightDomain = "page.tine.Tine.pages"

/// Hand a `tine://` route to the app's existing URL handler, exactly as if
/// iOS had opened the URL: tao's `application:openURL:options:` emits
/// `RunEvent::Opened`, and deep_links.rs `receive_url` queues it for the
/// frontend. Routes therefore never fork the `tine://` machinery.
func deliverTineRoute(_ url: URL) {
  DispatchQueue.main.async {
    let app = UIApplication.shared
    _ = app.delegate?.application?(app, open: url, options: [:])
  }
}

/// The S3 route for a Spotlight item: its identifier is the page key.
func tineSpotlightRoute(_ identifier: String) -> URL? {
  var allowed = CharacterSet.alphanumerics
  allowed.insert(charactersIn: "-._~")
  guard let encoded = identifier.addingPercentEncoding(withAllowedCharacters: allowed) else {
    return nil
  }
  return URL(string: "tine://page/\(encoded)")
}

/// Quick actions and Spotlight continuations reach the application delegate,
/// which is tao's runtime-declared `AppDelegate` class. tao implements neither
/// `application:performActionForShortcutItem:completionHandler:` nor a
/// Spotlight branch of `application:continueUserActivity:restorationHandler:`
/// (it handles only `webpageURL` and returns NO otherwise), so add the first
/// and wrap the second. Plugins initialize during Tauri's build, after tao
/// declared the class and before `UIApplicationMain` runs, so a cold launch
/// from a quick action or a Spotlight result also lands here.
enum RouteHooks {
  private static var installed = false
  private static var originalContinue: IMP?

  static func install() {
    guard !installed, let cls = NSClassFromString("AppDelegate") else { return }
    installed = true

    let shortcut = NSSelectorFromString("application:performActionForShortcutItem:completionHandler:")
    let shortcutBlock: @convention(block) (AnyObject, UIApplication, UIApplicationShortcutItem, @escaping @convention(block) (Bool) -> Void) -> Void = { _, _, item, completion in
      guard let text = item.userInfo?["url"] as? String, let url = URL(string: text) else {
        completion(false)
        return
      }
      deliverTineRoute(url)
      completion(true)
    }
    class_addMethod(cls, shortcut, imp_implementationWithBlock(shortcutBlock), "v@:@@@?")

    let cont = NSSelectorFromString("application:continueUserActivity:restorationHandler:")
    typealias ContinueFn = @convention(c) (AnyObject, Selector, UIApplication, NSUserActivity, AnyObject?) -> Bool
    if let method = class_getInstanceMethod(cls, cont) {
      originalContinue = method_getImplementation(method)
    }
    let continueBlock: @convention(block) (AnyObject, UIApplication, NSUserActivity, AnyObject?) -> Bool = { this, app, activity, handler in
      if activity.activityType == CSSearchableItemActionType,
         let identifier = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String,
         let url = tineSpotlightRoute(identifier) {
        deliverTineRoute(url)
        return true
      }
      guard let original = originalContinue else { return false }
      return unsafeBitCast(original, to: ContinueFn.self)(this, cont, app, activity, handler)
    }
    let imp = imp_implementationWithBlock(continueBlock)
    if originalContinue != nil {
      class_replaceMethod(cls, cont, imp, "B@:@@@?")
    } else {
      class_addMethod(cls, cont, imp, "B@:@@@?")
    }
  }
}

struct SpotlightEntry: Decodable {
  let id: String
  let title: String
  let excerpt: String
  let url: String
}

struct SpotlightUpdate: Decodable {
  let op: String
  let entries: [SpotlightEntry]?
  let ids: [String]?
}

final class NativeIntegrationsPlugin: Plugin {
  private var observing = false

  override init() {
    super.init()
    RouteHooks.install()
  }

  override func load(webview: WKWebView) {
    guard !observing else { return }
    observing = true
    let center = CFNotificationCenterGetDarwinNotifyCenter()
    let observer = Unmanaged.passUnretained(self).toOpaque()
    CFNotificationCenterAddObserver(
      center, observer,
      { _, observer, _, _, _ in
        guard let observer = observer else { return }
        let plugin = Unmanaged<NativeIntegrationsPlugin>.fromOpaque(observer).takeUnretainedValue()
        DispatchQueue.main.async { plugin.trigger("inboxChanged", data: JSObject()) }
      },
      tineInboxChangedNotification as CFString, nil, .deliverImmediately)
  }

  /// The share inbox root: `share-inbox` in the App Group container, which
  /// the Share Extension and the "Add to Tine journal" intent also write.
  /// Without the App Group (an unprovisioned build) the app's own
  /// Application Support directory keeps in-app producers working.
  @objc public func inboxDirectory(_ invoke: Invoke) {
    let fm = FileManager.default
    let base = fm.containerURL(forSecurityApplicationGroupIdentifier: tineAppGroup)
      ?? fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
    guard let root = base?.appendingPathComponent("share-inbox", isDirectory: true) else {
      invoke.reject("no share inbox location")
      return
    }
    do {
      try fm.createDirectory(at: root, withIntermediateDirectories: true)
      // Make the inbox's entry durable on every call, not only when this call
      // created it: existence proves nothing about the parent's metadata
      // (review round 2, R2-4; the producers do the same, ShareInbox.swift).
      let parent = root.deletingLastPathComponent().path
      let fd = open(parent, O_RDONLY)
      if fd < 0 { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
      defer { close(fd) }
      if fcntl(fd, F_FULLFSYNC) == -1 && fsync(fd) != 0 {
        throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
      }
      invoke.resolve(["path": root.path])
    } catch {
      invoke.reject(error.localizedDescription)
    }
  }

  @objc public func spotlight(_ invoke: Invoke) {
    let update: SpotlightUpdate
    do {
      update = try invoke.parseArgs(SpotlightUpdate.self)
    } catch {
      invoke.reject(error.localizedDescription)
      return
    }
    guard CSSearchableIndex.isIndexingAvailable() else {
      invoke.resolve()
      return
    }
    let index = CSSearchableIndex.default()
    let finish: (Error?) -> Void = { error in
      if let error = error { invoke.reject(error.localizedDescription) } else { invoke.resolve() }
    }
    switch update.op {
    case "replace":
      let items = (update.entries ?? []).map(Self.item)
      index.deleteSearchableItems(withDomainIdentifiers: [tineSpotlightDomain]) { error in
        if let error = error { finish(error); return }
        index.indexSearchableItems(items, completionHandler: finish)
      }
    case "upsert":
      index.indexSearchableItems((update.entries ?? []).map(Self.item), completionHandler: finish)
    case "delete":
      index.deleteSearchableItems(withIdentifiers: update.ids ?? [], completionHandler: finish)
    case "clear":
      index.deleteSearchableItems(withDomainIdentifiers: [tineSpotlightDomain], completionHandler: finish)
    default:
      invoke.reject("unknown Spotlight update \(update.op)")
    }
  }

  private static func item(_ entry: SpotlightEntry) -> CSSearchableItem {
    let attributes = CSSearchableItemAttributeSet(itemContentType: "public.text")
    attributes.title = entry.title
    attributes.contentDescription = entry.excerpt
    attributes.contentURL = URL(string: entry.url)
    return CSSearchableItem(
      uniqueIdentifier: entry.id, domainIdentifier: tineSpotlightDomain, attributeSet: attributes)
  }
}

@_cdecl("init_plugin_tine_native_integrations")
func initPlugin() -> Plugin {
  return NativeIntegrationsPlugin()
}
