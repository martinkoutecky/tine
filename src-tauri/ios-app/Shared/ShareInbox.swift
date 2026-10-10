import Foundation

/// The producer side of Tine's share inbox (ADR 0073), compiled into the app
/// target (the "Add to Tine journal" App Intent) and the Share Extension.
/// It writes ONLY the inbox in the App Group container, never the graph: the
/// running app ingests each item through its single graph writer
/// (src/shareIngest.ts) and removes it after the journal write reached disk.
///
/// Publication is crash-safe: everything is written into `.tmp-<id>/`, then
/// the directory is renamed to `<id>` in one step, so the app never sees a
/// partial item. An interrupted `.tmp-` directory is swept by the app after a
/// day (src-tauri/src/share_inbox.rs).
enum ShareInbox {
  static let appGroup = "group.page.tine.Tine"
  static let changedNotification = "page.tine.Tine.share-inbox-changed"

  struct Resource {
    /// Bytes to store, or a file to copy.
    let data: Data?
    let source: URL?
    /// File name inside the item, and the display name.
    let name: String
    let type: String
  }

  enum Failure: LocalizedError {
    case noContainer
    case empty
    var errorDescription: String? {
      switch self {
      case .noContainer: return "Tine's shared inbox is unavailable."
      case .empty: return "There was nothing to save."
      }
    }
  }

  static func root() throws -> URL {
    guard let container = FileManager.default.containerURL(
      forSecurityApplicationGroupIdentifier: appGroup)
    else { throw Failure.noContainer }
    return container.appendingPathComponent("share-inbox", isDirectory: true)
  }

  /// A plain, unique file name for a resource inside one item.
  private static func fileName(_ name: String, used: inout Set<String>) -> String {
    var base = name.replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "\\", with: "_")
      .trimmingCharacters(in: .whitespacesAndNewlines)
    if base.isEmpty || base == "." || base == ".." || base == "item.json" || base == "prepared.json" {
      base = "file"
    }
    var candidate = base
    var n = 1
    while used.contains(candidate) {
      candidate = "\(n)-\(base)"
      n += 1
    }
    used.insert(candidate)
    return candidate
  }

  /// Publish one item; returns its id.
  @discardableResult
  static func publish(text: String?, title: String?, url: String?, resources: [Resource]) throws -> String {
    let clean: (String?) -> String? = { value in
      guard let value = value, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
      return value
    }
    let text = clean(text), title = clean(title), url = clean(url)
    if text == nil && url == nil && resources.isEmpty { throw Failure.empty }
    let fm = FileManager.default
    let root = try root()
    try fm.createDirectory(at: root, withIntermediateDirectories: true)
    let id = UUID().uuidString.lowercased()
    let tmp = root.appendingPathComponent(".tmp-\(id)", isDirectory: true)
    try fm.createDirectory(at: tmp, withIntermediateDirectories: false)
    do {
      var used = Set<String>()
      var described: [[String: String]] = []
      for resource in resources {
        let file = fileName(resource.name, used: &used)
        let target = tmp.appendingPathComponent(file)
        if let data = resource.data {
          try data.write(to: target, options: .atomic)
        } else if let source = resource.source {
          try fm.copyItem(at: source, to: target)
        } else {
          continue
        }
        described.append(["file": file, "name": resource.name, "type": resource.type])
      }
      var item: [String: Any] = [
        "version": 1,
        "created": Int64(Date().timeIntervalSince1970 * 1000),
        "resources": described,
      ]
      if let text = text { item["text"] = text }
      if let title = title { item["title"] = title }
      if let url = url { item["url"] = url }
      let json = try JSONSerialization.data(withJSONObject: item)
      try json.write(to: tmp.appendingPathComponent("item.json"), options: .atomic)
      try fm.moveItem(at: tmp, to: root.appendingPathComponent(id, isDirectory: true))
    } catch {
      try? fm.removeItem(at: tmp)
      throw error
    }
    CFNotificationCenterPostNotification(
      CFNotificationCenterGetDarwinNotifyCenter(),
      CFNotificationName(changedNotification as CFString), nil, nil, true)
    return id
  }
}
