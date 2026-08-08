import SwiftRs
import Tauri
import UIKit
import UniformTypeIdentifiers

final class GraphFolderPickerPlugin: Plugin, UIDocumentPickerDelegate {
  private static let markerName = ".tine-container"

  private var documentsURL: URL?
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

  @objc public func pickGraphFolder(_ invoke: Invoke) {
    guard let documentsURL else {
      invoke.reject(
        "Couldn't prepare Tine's Documents container. \(containerSetupError ?? "Unknown error")"
      )
      return
    }

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
      let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
      picker.delegate = self
      picker.directoryURL = documentsURL
      picker.allowsMultipleSelection = false
      picker.modalPresentationStyle = .fullScreen
      viewController.present(picker, animated: true)
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
    let documentsPath = documentsURL.path
    let selectedPath = resolved.path
    let isInsideDocuments = selectedPath == documentsPath
      || selectedPath.hasPrefix(documentsPath + "/")

    if isInsideDocuments {
      invoke.resolve(["status": "picked", "path": selectedPath])
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
