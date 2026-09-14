import { render } from "solid-js/web";
import { PdfViewer } from "../../../src/components/PdfViewer";
import { backend } from "../../../src/backend";
import { activatePdfOwnership } from "../../../src/pdfOwnership";
import "../../../src/styles/inter.css";
import "../../../src/styles/theme.css";
import "../../../src/styles/app.css";
const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
  "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>",
  "<< /Length 0 >>\nstream\n\nendstream"];
let pdf = "%PDF-1.4\n"; const offsets = [0];
objects.forEach((body, i) => { offsets.push(pdf.length); pdf += `${i + 1} 0 obj\n${body}\nendobj\n`; });
const xref = pdf.length;
pdf += `xref\n0 5\n0000000000 65535 f \n${offsets.slice(1).map(n => String(n).padStart(10,"0") + " 00000 n ").join("\n")}\ntrailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
Object.assign(backend(), { openPdf: async () => ({highlights: [], page: 1, scale: 1}), readAsset: async () => new TextEncoder().encode(pdf) });
const owner = activatePdfOwnership("/pdf-toolbar-probe");
(window as any).pdfProbe = { notes: 0, close: 0 };
render(() => <div class="pdf-pane pdf-route-pane" style="width:100%;height:100vh"><PdfViewer
  filename="android-route.pdf" label="Android route PDF" owner={owner}
  onOpenNotes={() => (window as any).pdfProbe.notes++}
  onClose={() => (window as any).pdfProbe.close++}/></div>, document.getElementById("root")!);
