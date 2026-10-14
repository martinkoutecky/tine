import { expect, vi } from "vitest";

/** Exercise the query's native disclosure and a named action. */
export async function openCalmQuery(root: HTMLElement, action = "Edit"): Promise<HTMLElement> {
  const trigger = await vi.waitFor(() => {
    const found = root.querySelector<HTMLElement>('[aria-label="Query actions"]');
    expect(found).not.toBeNull();
    return found!;
  });
  const details = trigger.closest("details")!;
  if (!details.open) trigger.click();
  const button = await vi.waitFor(() => {
    const menu = trigger.ownerDocument.getElementById(trigger.getAttribute("aria-controls")!);
    const found = [...(menu?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((b) => b.textContent === action);
    expect(found).toBeDefined();
    return found!;
  });
  button.click();
  return await vi.waitFor(() => {
    const found = root.querySelector<HTMLElement>(".qc-panel");
    expect(found).not.toBeNull();
    return found!;
  });
}
