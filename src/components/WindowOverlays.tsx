import { Show, Suspense, lazy, type JSX } from "solid-js";
import { QuickSwitcher } from "./QuickSwitcher";
import { ContextMenu } from "./ContextMenu";
import { Toasts, Lightbox } from "./Toasts";
import { AudioOverlay } from "./AudioOverlay";
import { HelpPopup } from "./HelpShortcuts";
import { DatePicker } from "./DatePicker";
import { FormulaEditor } from "./FormulaEditor";
import { PageProps } from "./PageProps";
import { ExportModal } from "./ExportModal";
import { PdfExportDialog } from "./PdfExportDialog";
import { QueryExportDialog } from "./QueryExportDialog";
import { GraphNamePrompt } from "./GraphNamePrompt";
import { FailureBoundary } from "./FailureBoundary";
import { DrawerBackground } from "./MobileDrawerShell";
import { queryExportRequest, settingsOpen } from "../ui";
import { overlayWindowId } from "../transientLayers";

const Settings = lazy(() => import("./Settings").then((module) => ({ default: module.Settings })));

/**
 * The app-level overlays (search, menus, dialogs, settings, toasts) of ONE Tine
 * window (OG-MULTIWINDOW P1). Their state is app-wide, so exactly one window
 * renders them: the window the user is in, held while one of them is open
 * (`overlayWindowId`, src/transientLayers.ts). Main passes its main-only
 * layers as slots so the stacking order stays the one it always had.
 */
export function WindowOverlays(props: {
  windowId: string;
  deepLink?: JSX.Element;
  keyboardToolbar?: JSX.Element;
  recovery?: JSX.Element;
  welcome?: JSX.Element;
}): JSX.Element {
  const here = () => overlayWindowId() === props.windowId;
  return (
    <>
      <Show when={here()}><FailureBoundary region="Search"><QuickSwitcher /></FailureBoundary></Show>
      {props.deepLink}
      <Show when={here()}>
        <FailureBoundary region="The context menu"><ContextMenu /></FailureBoundary>
        <FailureBoundary region="The date picker"><DatePicker /></FailureBoundary>
        <FailureBoundary region="The formula editor"><FormulaEditor /></FailureBoundary>
      </Show>
      {props.keyboardToolbar}
      <Show when={here()}>
        <FailureBoundary region="Page properties"><PageProps /></FailureBoundary>
        <FailureBoundary region="Export"><ExportModal /></FailureBoundary>
      </Show>
      {props.recovery}
      <Show when={here()}>
        <FailureBoundary region="PDF export"><PdfExportDialog /></FailureBoundary>
        <FailureBoundary region="Query export"><QueryExportDialog request={queryExportRequest} /></FailureBoundary>
        <FailureBoundary region="Graph creation"><GraphNamePrompt /></FailureBoundary>
        <Show when={settingsOpen()}>
          <Suspense>
            <FailureBoundary region="Settings"><Settings /></FailureBoundary>
          </Suspense>
        </Show>
        <FailureBoundary region="Help"><HelpPopup /></FailureBoundary>
      </Show>
      {props.welcome}
      <Show when={here()}>
        <DrawerBackground class="drawer-floating-background" blockedBy="any">
          <Toasts />
        </DrawerBackground>
        <FailureBoundary region="This image"><Lightbox /></FailureBoundary>
        <FailureBoundary region="This audio"><AudioOverlay /></FailureBoundary>
      </Show>
    </>
  );
}
