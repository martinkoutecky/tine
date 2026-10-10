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
/// - A share is saved whole or not at all (review round 1, finding 6): an
///   attachment that fails to load, a file that is not an image, a second
///   different web link or an unsupported attachment refuses the share with
///   a message; OG skips them. A repeated identical link is kept once.
/// - Several text attachments are joined one per line (OG keeps the last).
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

  /// A share Tine refuses whole, with the reason shown to the user.
  private struct Refusal: LocalizedError {
    let errorDescription: String?
    init(_ message: String) { errorDescription = message }
  }

  private static func tooLarge(_ name: String) -> Refusal {
    Refusal("\(name) is larger than 64 MiB.")
  }

  private static func imageResource(_ url: URL) throws -> ShareInbox.Resource {
    guard isImage(url) else {
      throw Refusal("Tine saves text, links and images; \(url.lastPathComponent) is another kind of file.")
    }
    // The 64 MiB limit is checked before the file is read (review round 2,
    // R2-6), and again on what was read when the size was not known.
    if let size = ShareInbox.fileSize(url), size > ShareInbox.maxResourceBytes { throw tooLarge(url.lastPathComponent) }
    let data = try Data(contentsOf: url, options: .mappedIfSafe)
    return try sized(.init(data: data, source: nil, name: url.lastPathComponent, type: mimeType(url)))
  }

  /// `resource`, unless its bytes are over the limit.
  private static func sized(_ resource: ShareInbox.Resource) throws -> ShareInbox.Resource {
    if (resource.data?.count ?? 0) > ShareInbox.maxResourceBytes { throw tooLarge(resource.name) }
    return resource
  }

  /// Every attachment, or a refusal: nothing is saved in part.
  private func collect() async throws -> (text: String?, webURL: String?, resources: [ShareInbox.Resource]) {
    var texts: [String] = []
    var webURL: String?
    var resources: [ShareInbox.Resource] = []
    let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
    // At most 32 files: the 33rd is refused before it is loaded (review
    // round 2, R2-6), and the share with it.
    let room: () throws -> Void = {
      if resources.count >= ShareInbox.maxResources {
        throw Refusal("Tine saves at most \(ShareInbox.maxResources) files from one share; this one had more.")
      }
    }
    for item in items {
      for attachment in item.attachments ?? [] {
        if attachment.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
          let loaded = try await attachment.loadItem(forTypeIdentifier: UTType.url.identifier, options: nil)
          let url: URL
          switch loaded {
          case let value as URL: url = value
          case let data as Data:
            guard let value = URL(dataRepresentation: data, relativeTo: nil) else {
              throw Refusal("A shared link couldn't be read.")
            }
            url = value
          default: throw Refusal("A shared link couldn't be read.")
          }
          if url.isFileURL {
            try room()
            resources.append(try Self.imageResource(url))
          } else if webURL == nil || webURL == url.absoluteString {
            webURL = url.absoluteString
          } else {
            throw Refusal("Tine saves one web link per share; this share had several.")
          }
        } else if attachment.hasItemConformingToTypeIdentifier(UTType.text.identifier) {
          let loaded = try await attachment.loadItem(forTypeIdentifier: UTType.text.identifier, options: nil)
          switch loaded {
          case let value as String: texts.append(value)
          case let value as NSAttributedString: texts.append(value.string)
          case let data as Data:
            guard let value = String(data: data, encoding: .utf8) else {
              throw Refusal("Shared text couldn't be read.")
            }
            texts.append(value)
          default: throw Refusal("Shared text couldn't be read.")
          }
        } else if attachment.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
          try room()
          let loaded = try await attachment.loadItem(forTypeIdentifier: UTType.image.identifier, options: nil)
          switch loaded {
          case let image as UIImage:
            guard let data = image.pngData() else { throw Refusal("A shared image couldn't be read.") }
            resources.append(try Self.sized(.init(data: data, source: nil, name: Self.timestampName("png"), type: "image/png")))
          case let url as URL:
            resources.append(try Self.imageResource(url))
          case let data as Data:
            resources.append(try Self.sized(.init(data: data, source: nil, name: Self.timestampName("png"), type: "image/png")))
          default:
            throw Refusal("A shared image couldn't be read.")
          }
        } else {
          throw Refusal("Tine saves text, links and images; this share had something else.")
        }
      }
    }
    let text = texts.filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }.joined(separator: "\n")
    return (text.isEmpty ? nil : text, webURL, resources)
  }

  private func save() async {
    let message: String
    var saved = false
    do {
      let share = try await collect()
      try ShareInbox.publish(text: share.text, title: nil, url: share.webURL, resources: share.resources)
      message = "Saved to Tine"
      saved = true
    } catch {
      message = "Couldn't save to Tine: \(error.localizedDescription) Nothing was saved."
    }
    await MainActor.run { self.label.text = message }
    try? await Task.sleep(nanoseconds: saved ? 700_000_000 : 3_000_000_000)
    await MainActor.run {
      self.extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
    }
  }
}
