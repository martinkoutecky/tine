import AppIntents
import Foundation
import UIKit

/// Tine's App Intents (I4), compiled into the app target so the system can
/// extract their metadata. Routes go through the app's existing `tine://`
/// URL handler (deep_links.rs `receive_url`); text goes to the share inbox,
/// never to the graph directly (ADR 0073).

private func tineRoute(_ path: String, query: [URLQueryItem] = []) -> URL? {
  var components = URLComponents()
  components.scheme = "tine"
  components.host = path
  components.queryItems = query.isEmpty ? nil : query
  return components.url
}

/// Hand a route to tao's URL handler, as if iOS had opened it.
@MainActor
private func openTineRoute(_ url: URL) {
  let app = UIApplication.shared
  _ = app.delegate?.application?(app, open: url, options: [:])
}

@available(iOS 16.0, *)
struct AddToTineJournalIntent: AppIntent {
  static var title: LocalizedStringResource = "Add to Tine journal"
  static var description = IntentDescription(
    "Adds the text as a new block at the bottom of today's journal in Tine.")
  static var openAppWhenRun: Bool = false

  @Parameter(title: "Text")
  var text: String

  func perform() async throws -> some IntentResult & ProvidesDialog {
    try ShareInbox.publish(text: text, title: nil, url: nil, resources: [])
    return .result(dialog: "Saved to Tine")
  }
}

@available(iOS 16.0, *)
struct OpenTinePageIntent: AppIntent {
  static var title: LocalizedStringResource = "Open page"
  static var description = IntentDescription("Opens a page of the current graph in Tine.")
  static var openAppWhenRun: Bool = true

  @Parameter(title: "Page")
  var page: String

  @MainActor
  func perform() async throws -> some IntentResult {
    var allowed = CharacterSet.alphanumerics
    allowed.insert(charactersIn: "-._~")
    if let encoded = page.addingPercentEncoding(withAllowedCharacters: allowed),
       let url = URL(string: "tine://page/\(encoded)") {
      openTineRoute(url)
    }
    return .result()
  }
}

@available(iOS 16.0, *)
struct SearchTineIntent: AppIntent {
  static var title: LocalizedStringResource = "Search Tine"
  static var description = IntentDescription("Opens Tine's search, optionally with a query.")
  static var openAppWhenRun: Bool = true

  @Parameter(title: "Query")
  var query: String?

  @MainActor
  func perform() async throws -> some IntentResult {
    let items = (query?.isEmpty == false) ? [URLQueryItem(name: "q", value: query)] : []
    if let url = tineRoute("search", query: items) { openTineRoute(url) }
    return .result()
  }
}

@available(iOS 16.0, *)
struct TineShortcuts: AppShortcutsProvider {
  static var appShortcuts: [AppShortcut] {
    AppShortcut(
      intent: AddToTineJournalIntent(),
      phrases: ["Add to \(.applicationName) journal", "Capture in \(.applicationName)"])
    AppShortcut(intent: SearchTineIntent(), phrases: ["Search \(.applicationName)"])
    AppShortcut(intent: OpenTinePageIntent(), phrases: ["Open a page in \(.applicationName)"])
  }
}
