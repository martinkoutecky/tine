/** GH #622: a native chooser, camera or file picker is a full-screen Android
 * activity, so the WebView reports the document hidden while the user picks.
 * That hide belongs to the edit in progress, not to leaving the app: the
 * background flush still saves, but must not end the edit the picker will
 * insert into. Holds nest; each release is idempotent. */
let held = 0;

export function holdExternalActivity(): () => void {
  held++;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    held--;
  };
}

export function externalActivityHeld(): boolean {
  return held > 0;
}
