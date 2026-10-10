import UIKit
import UniformTypeIdentifiers

/// Tine's Share Extension (I2). Transcribed from OG's
/// `ios/App/ShareViewController/ShareViewController.swift` (logseq/og
/// 6e7afa8eb): the same attachment classification (URL: a file is a
/// resource, a web URL is the link; text is the item text; an image is a
/// resource, a `UIImage` stored as `yyyy-MM-dd-HH-mm-ss.png`).
///
/// Deliberate differences (ADR 0073):
/// - OG hands the payload to the app by opening `logseq-og://shared?payload=`
///   through a private responder-chain `openURL`. Tine publishes it to the
///   App Group share inbox instead (`ShareInbox.publish`) and does not open
///   the app: the item is added to the bottom of today's journal the next
///   time Tine runs or comes to the foreground, and is never lost if the
///   write fails.
/// - It shows a short "Saved to Tine" confirmation (OG closes after 0.2 s
///   without one).
/// - Movies and arbitrary files are not offered (the dossier scope is text,
///   links and images), and one web link per share.
final class ShareViewController: UIViewController {
  private let label = UILabel()

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = UIColor.systemBackground.withAlphaComponent(0.92)
    label.translatesAutoresizingMaskIntoConstraints = false
    label.font = UIFont.preferredFont(forTextStyle: .headline)
    label.textAlignment = .center
    label.numberOfLines = 0
    label.text = "Saving to Tine…"
    view.addSubview(label)
    NSLayoutConstraint.activate([
      label.centerXAnchor.constraint(equalTo: view.centerXAnchor),
      label.centerYAnchor.constraint(equalTo: view.centerYAnchor),
      label.leadingAnchor.constraint(greaterThanOrEqualTo: view.leadingAnchor, constant: 24),
    ])
    Task { await save() }
  }

  private static func timestampName(_ ext: String) -> String {
    let formatter = DateFormatter()
    formatter.dateFormat = "yyyy-MM-dd-HH-mm-ss"
    return formatter.string(from: Date()) + "." + ext
  }

  private static func mimeType(_ url: URL) -> String {
    UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
  }

  private static func isImage(_ url: URL) -> Bool {
    UTType(filenameExtension: url.pathExtension)?.conforms(to: .image) ?? false
  }

  private func save() async {
    var text: String?
    var webURL: String?
    var resources: [ShareInbox.Resource] = []
    let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
    for item in items {
      for attachment in item.attachments ?? [] {
        do {
          if attachment.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
            let loaded = try await attachment.loadItem(forTypeIdentifier: UTType.url.identifier, options: nil)
            guard let url = loaded as? URL else { continue }
            if url.isFileURL {
              if Self.isImage(url) {
                resources.append(.init(data: try Data(contentsOf: url), source: nil,
                                       name: url.lastPathComponent, type: Self.mimeType(url)))
              }
            } else if webURL == nil {
              webURL = url.absoluteString
            }
          } else if attachment.hasItemConformingToTypeIdentifier(UTType.text.identifier) {
            let loaded = try await attachment.loadItem(forTypeIdentifier: UTType.text.identifier, options: nil)
            if let value = loaded as? String { text = value }
          } else if attachment.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
            let loaded = try await attachment.loadItem(forTypeIdentifier: UTType.image.identifier, options: nil)
            switch loaded {
            case let image as UIImage:
              if let data = image.pngData() {
                resources.append(.init(data: data, source: nil, name: Self.timestampName("png"), type: "image/png"))
              }
            case let url as URL:
              resources.append(.init(data: try Data(contentsOf: url), source: nil,
                                     name: url.lastPathComponent, type: Self.mimeType(url)))
            case let data as Data:
              resources.append(.init(data: data, source: nil, name: Self.timestampName("png"), type: "image/png"))
            default:
              break
            }
          }
        } catch {
          continue
        }
      }
    }
    let message: String
    var saved = false
    do {
      try ShareInbox.publish(text: text, title: nil, url: webURL, resources: resources)
      message = "Saved to Tine"
      saved = true
    } catch {
      message = "Couldn't save to Tine: \(error.localizedDescription)"
    }
    await MainActor.run { self.label.text = message }
    try? await Task.sleep(nanoseconds: saved ? 700_000_000 : 2_000_000_000)
    await MainActor.run {
      self.extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
    }
  }
}
