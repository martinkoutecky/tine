import SwiftRs
import Tauri
import UIKit
import UniformTypeIdentifiers

final class GraphFolderPickerPlugin: Plugin, UIDocumentPickerDelegate {
  private static let markerName = ".tine-container"
  private static let iCloudContainerIdentifier = "iCloud.page.tine.Tine"
  private static let materializationTimeout: TimeInterval = 120

  private var documentsURL: URL?
  private var iCloudDocumentsURL: URL?
  private var containerSetupError: String?
  private var pendingInvoke: Invoke?

  override init() {
    super.init()
    do {
      documentsURL = try Self.prepareDocumentsContainer()
    } catch {
      containerSetupError = error.localizedDescription
    }
  }

  private static func prepareDocumentsContainer() throws -> URL {
    guard let documents = FileManager.default.urls(
      for: .documentDirectory,
      in: .userDomainMask
    ).first else {
      throw NSError(
        domain: "page.tine.app.folder-picker",
        code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Tine's Documents container is unavailable."]
      )
    }

    try FileManager.default.createDirectory(
      at: documents,
      withIntermediateDirectories: true,
      attributes: nil
    )

    let marker = documents.appendingPathComponent(markerName, isDirectory: false)
    if !FileManager.default.fileExists(atPath: marker.path) {
      try Data().write(to: marker, options: .atomic)
    }
    return documents.standardizedFileURL.resolvingSymlinksInPath()
  }

  private static func prepareICloudDocumentsContainer() throws -> URL? {
    guard FileManager.default.ubiquityIdentityToken != nil else { return nil }
    guard let container = FileManager.default.url(
      forUbiquityContainerIdentifier: iCloudContainerIdentifier
    ) else { return nil }

    let documents = container.appendingPathComponent("Documents", isDirectory: true)
    try FileManager.default.createDirectory(
      at: documents,
      withIntermediateDirectories: true,
      attributes: nil
    )
    let marker = documents.appendingPathComponent(markerName, isDirectory: false)
    if !FileManager.default.fileExists(atPath: marker.path) {
      try Data().write(to: marker, options: .atomic)
    }
    return documents.standardizedFileURL.resolvingSymlinksInPath()
  }

  private static func isInside(_ selected: URL, container: URL) -> Bool {
    let selectedPath = selected.standardizedFileURL.resolvingSymlinksInPath().path
    let containerPath = container.standardizedFileURL.resolvingSymlinksInPath().path
    return selectedPath == containerPath || selectedPath.hasPrefix(containerPath + "/")
  }

  /// App updates may preserve Documents while changing the absolute sandbox
  /// UUID. Recover only paths that have the exact iOS app-container shape and
  /// whose Documents-relative target already exists in this installation.
  private static func rebaseStaleLocalContainerPath(
    _ selected: URL,
    into documents: URL
  ) -> URL? {
    let components = selected.standardizedFileURL.pathComponents
    guard components.count >= 6 else { return nil }

    for index in 0...(components.count - 6) {
      guard components[index] == "Containers",
            components[index + 1] == "Data",
            components[index + 2] == "Application",
            UUID(uuidString: components[index + 3]) != nil,
            components[index + 4] == "Documents" else { continue }

      let relative = components.dropFirst(index + 5)
      guard !relative.isEmpty else { return documents }
      let candidate = relative.reduce(documents) { partial, component in
        partial.appendingPathComponent(component, isDirectory: true)
      }.standardizedFileURL.resolvingSymlinksInPath()
      guard isInside(candidate, container: documents) else { return nil }
      var isDirectory: ObjCBool = false
      guard FileManager.default.fileExists(atPath: candidate.path, isDirectory: &isDirectory),
            isDirectory.boolValue else { return nil }
      return candidate
    }
    return nil
  }

  private static func downloadUbiquitousContents(at root: URL) throws {
    let fileManager = FileManager.default
    if fileManager.isUbiquitousItem(at: root) {
      try fileManager.startDownloadingUbiquitousItem(at: root)
    }
    let keys: [URLResourceKey] = [
      .isDirectoryKey,
      .isUbiquitousItemKey,
      .ubiquitousItemDownloadingStatusKey,
      .ubiquitousItemDownloadingErrorKey,
    ]
    let deadline = Date().addingTimeInterval(materializationTimeout)

    while true {
      var pending = 0
      guard let enumerator = fileManager.enumerator(
        at: root,
        includingPropertiesForKeys: keys,
        options: [],
        errorHandler: { _, _ in false }
      ) else {
        throw NSError(
          domain: "page.tine.app.folder-picker",
          code: 3,
          userInfo: [NSLocalizedDescriptionKey: "TineOutline couldn't enumerate the iCloud graph."]
        )
      }

      for case let item as URL in enumerator {
        let values = try item.resourceValues(forKeys: Set(keys))
        if let error = values.ubiquitousItemDownloadingError { throw error }
        guard values.isUbiquitousItem == true else { continue }
        if values.ubiquitousItemDownloadingStatus != .current {
          try fileManager.startDownloadingUbiquitousItem(at: item)
          pending += 1
        }
      }

      if pending == 0 { return }
      if Date() >= deadline {
        throw NSError(
          domain: "page.tine.app.folder-picker",
          code: 4,
          userInfo: [
            NSLocalizedDescriptionKey:
              "The iCloud graph is still downloading. Keep TineOutline open and try again."
          ]
        )
      }
      Thread.sleep(forTimeInterval: 0.25)
    }
  }

  @available(iOS 14.0, *)
  @objc public func pickGraphFolder(_ invoke: Invoke) {
    guard let documentsURL else {
      invoke.reject(
        "Couldn't prepare Tine's Documents container. \(containerSetupError ?? "Unknown error")"
      )
      return
    }

    DispatchQueue.global(qos: .userInitiated).async {
      let iCloudDocuments = try? Self.prepareICloudDocumentsContainer()
      DispatchQueue.main.async {
        guard self.pendingInvoke == nil else {
          invoke.reject("A folder picker is already open.")
          return
        }
        guard let viewController = self.manager.viewController else {
          invoke.reject("Tine couldn't present the iOS folder picker.")
          return
        }

        self.pendingInvoke = invoke
        self.iCloudDocumentsURL = iCloudDocuments
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
        picker.delegate = self
        picker.directoryURL = iCloudDocuments ?? documentsURL
        picker.allowsMultipleSelection = false
        picker.modalPresentationStyle = .fullScreen
        viewController.present(picker, animated: true)
      }
    }
  }

  @objc public func prepareGraphFolder(_ invoke: Invoke) {
    struct Args: Decodable { let path: String }

    do {
      let args = try invoke.parseArgs(Args.self)
      let selected = URL(fileURLWithPath: args.path, isDirectory: true)
      DispatchQueue.global(qos: .userInitiated).async {
        do {
          guard let local = self.documentsURL else {
            throw NSError(
              domain: "page.tine.app.folder-picker",
              code: 2,
              userInfo: [NSLocalizedDescriptionKey: "TineOutline's local Documents container is unavailable."]
            )
          }
          if Self.isInside(selected, container: local) {
            invoke.resolve(["status": "ready", "location": "local", "path": selected.path])
            return
          }
          if let rebased = Self.rebaseStaleLocalContainerPath(selected, into: local) {
            invoke.resolve(["status": "ready", "location": "local", "path": rebased.path])
            return
          }
          if let iCloud = try Self.prepareICloudDocumentsContainer(),
             Self.isInside(selected, container: iCloud) {
            try Self.downloadUbiquitousContents(at: selected)
            invoke.resolve(["status": "ready", "location": "icloud", "path": selected.path])
            return
          }
          invoke.resolve(["status": "refused"])
        } catch {
          invoke.reject(error.localizedDescription)
        }
      }
    } catch {
      invoke.reject(error.localizedDescription)
    }
  }

  func documentPicker(
    _ controller: UIDocumentPickerViewController,
    didPickDocumentsAt urls: [URL]
  ) {
    guard let invoke = pendingInvoke else { return }
    pendingInvoke = nil
    guard let selected = urls.first, let documentsURL else {
      invoke.reject("The iOS folder picker returned no folder.")
      return
    }

    let resolved = selected.standardizedFileURL.resolvingSymlinksInPath()
    let isInsideLocal = Self.isInside(resolved, container: documentsURL)
    let isInsideICloud = iCloudDocumentsURL.map {
      Self.isInside(resolved, container: $0)
    } ?? false

    if isInsideLocal || isInsideICloud {
      invoke.resolve(["status": "picked", "path": resolved.path])
    } else {
      invoke.resolve(["status": "refused"])
    }
  }

  func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
    guard let invoke = pendingInvoke else { return }
    pendingInvoke = nil
    invoke.resolve(["status": "cancelled"])
  }
}

@_cdecl("init_plugin_tine_ios_folder_picker")
func initPlugin() -> Plugin {
  return GraphFolderPickerPlugin()
}
