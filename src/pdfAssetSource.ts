/** An asset snapshot's metadata and bounded, read-only byte ranges. Native
 * reads capture the graph binding and reject a changed source file. */
export interface PdfAssetSource {
  length: number;
  read(begin: number, end: number): Promise<Uint8Array>;
}

export interface PdfAssetInfo { length: number; version: string }
