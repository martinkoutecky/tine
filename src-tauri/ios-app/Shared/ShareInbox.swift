import Foundation

/// The producer side of Tine's share inbox (ADR 0073), compiled into the app
/// target (the "Add to Tine journal" App Intent) and the Share Extension.
/// It writes ONLY the inbox in the App Group container, never the graph: the
/// running app ingests each item through its single graph writer
/// (src/shareIngest.ts) and removes it after the journal write reached disk.
///
/// Publication is atomic and durable before anyone reports success (review
/// round 1, finding 4): everything is written into `.tmp-<id>/` with each
/// file synced (`F_FULLFSYNC`), the directory synced, renamed to `<id>` in
/// one step, then the inbox synced; any error propagates and nothing is
/// reported saved. The app never sees a partial item. An interrupted `.tmp-`
/// directory is swept by the app after a day (src-tauri/src/share_inbox.rs).
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

  /// Must match share_inbox.rs (`MAX_RESOURCES`).
  static let maxResources = 32
  /// The largest file one share may carry (as on Android, ShareIntake.kt).
  static let maxResourceBytes = 64 * 1024 * 1024

  enum Failure: LocalizedError {
    case noContainer
    case empty
    case tooMany(Int)
    case tooLarge(String)
    case io(String, Int32)
    var errorDescription: String? {
      switch self {
      case .noContainer: return "Tine's shared inbox is unavailable."
      case .empty: return "There was nothing to save."
      case .tooMany(let count):
        return "Tine saves at most \(ShareInbox.maxResources) files from one share; this one had \(count)."
      case .tooLarge(let name): return "\(name) is larger than 64 MiB."
      case .io(let what, let code): return "\(what) failed: \(String(cString: strerror(code)))"
      }
    }
  }

  /// Flush `fd` to stable storage: `F_FULLFSYNC`, or `fsync` where the file
  /// system does not support it.
  private static func fullSync(_ fd: Int32, _ what: String) throws {
    if fcntl(fd, F_FULLFSYNC) == -1 && fsync(fd) != 0 {
      throw Failure.io(what, errno)
    }
  }

  /// Make a directory's entries durable.
  private static func syncDirectory(_ url: URL) throws {
    let fd = open(url.path, O_RDONLY)
    if fd < 0 { throw Failure.io("opening \(url.lastPathComponent)", errno) }
    defer { close(fd) }
    try fullSync(fd, "syncing \(url.lastPathComponent)")
  }

  /// Create `url` holding `data`, synced before it returns.
  private static func writeSynced(_ data: Data, to url: URL) throws {
    let fd = open(url.path, O_WRONLY | O_CREAT | O_EXCL, 0o600)
    if fd < 0 { throw Failure.io("creating \(url.lastPathComponent)", errno) }
    let handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
    try handle.write(contentsOf: data)
    try fullSync(fd, "syncing \(url.lastPathComponent)")
    try handle.close()
  }

  static func root() throws -> URL {
    guard let container = FileManager.default.containerURL(
      forSecurityApplicationGroupIdentifier: appGroup)
    else { throw Failure.noContainer }
    return container.appendingPathComponent("share-inbox", isDirectory: true)
  }

  /// Size of the file at `url` without reading it, when the file system says.
  static func fileSize(_ url: URL) -> Int? {
    try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize
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
    if resources.count > maxResources { throw Failure.tooMany(resources.count) }
    let fm = FileManager.default
    let root = try root()
    // Every publication makes the inbox's own entry durable, whoever created
    // it (the app's plugin, an earlier share cut short): an existing
    // directory proves nothing about its parent (review round 2, R2-4).
    try fm.createDirectory(at: root, withIntermediateDirectories: true)
    try syncDirectory(root.deletingLastPathComponent())
    let id = UUID().uuidString.lowercased()
    let tmp = root.appendingPathComponent(".tmp-\(id)", isDirectory: true)
    try fm.createDirectory(at: tmp, withIntermediateDirectories: false)
    do {
      var used = Set<String>()
      var described: [[String: String]] = []
      for resource in resources {
        let file = fileName(resource.name, used: &used)
        let target = tmp.appendingPathComponent(file)
        let data: Data
        if let bytes = resource.data {
          data = bytes
        } else if let source = resource.source {
          // Refused before loading when the size is known (review round 2, R2-6).
          if let size = fileSize(source), size > maxResourceBytes { throw Failure.tooLarge(resource.name) }
          data = try Data(contentsOf: source, options: .mappedIfSafe)
        } else {
          throw Failure.io("reading \(resource.name)", EIO)
        }
        if data.count > maxResourceBytes { throw Failure.tooLarge(resource.name) }
        try writeSynced(data, to: target)
        described.append(["file": file, "name": resource.name, "type": resource.type])
      }
      var item: [String: Any] = [
        "version": 1,
        "source": "ios",
        "created": Int64(Date().timeIntervalSince1970 * 1000),
        "resources": described,
      ]
      if let text = text { item["text"] = text }
      if let title = title { item["title"] = title }
      if let url = url { item["url"] = url }
      let json = try JSONSerialization.data(withJSONObject: item)
      try writeSynced(json, to: tmp.appendingPathComponent("item.json"))
      try syncDirectory(tmp)
      let target = root.appendingPathComponent(id, isDirectory: true)
      if rename(tmp.path, target.path) != 0 { throw Failure.io("publishing the item", errno) }
      try syncDirectory(root)
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
