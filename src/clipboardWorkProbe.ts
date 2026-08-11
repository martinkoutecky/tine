// F3's clipboard work receipt. The store calls this only from Vite test-mode
// branches, so it cannot alter clipboard authority, persistence scheduling, or
// production hot-path work.

export interface ClipboardWorkForTest {
  label: string | null;
  public_markdown_visits: number;
  public_markdown_raw_bytes: number;
  private_payload_visits: number;
  private_payload_raw_bytes: number;
  prepared_destination_nodes: number;
  allocated_destination_nodes: number;
  source_retirement_phases: number;
  resolve_blocks_phases: number;
  final_identity_guard_phases: number;
  target_insertion_phases: number;
}

let work: ClipboardWorkForTest = emptyClipboardWork(null);

function emptyClipboardWork(label: string | null): ClipboardWorkForTest {
  return {
    label,
    public_markdown_visits: 0,
    public_markdown_raw_bytes: 0,
    private_payload_visits: 0,
    private_payload_raw_bytes: 0,
    prepared_destination_nodes: 0,
    allocated_destination_nodes: 0,
    source_retirement_phases: 0,
    resolve_blocks_phases: 0,
    final_identity_guard_phases: 0,
    target_insertion_phases: 0,
  };
}

/** Begin a labelled test receipt. Production callers never enable one. */
export function __resetClipboardWorkForTest(label: string | null = null): void {
  work = emptyClipboardWork(label);
}

/** Read the current labelled clipboard work receipt. */
export function __clipboardWorkForTest(): ClipboardWorkForTest {
  return { ...work };
}

export function recordClipboardWorkForTest(
  metric: Exclude<keyof ClipboardWorkForTest, "label">,
  count = 1,
): void {
  work[metric] += count;
}
