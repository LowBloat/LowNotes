import type { ExportOptions } from './export-assets';
export { parseDocument, type DocumentBlock, type TextSpan } from './document-model';
export type { ExportOptions, ExportImage } from './export-assets';
export { createDocx } from './docx-export';
export { createPdf, type PdfFonts } from './pdf-export';
export type ExportFormat = 'docx' | 'pdf';

export function exportFileName(path: string, format: ExportFormat): string {
  const name = (path.split(/[\\/]/).pop() || 'document').replace(/\.(md|markdown)$/i, '').replace(/[<>:"/\\|?*\x00-\x1f]/g, '-').trim();
  return `${name || 'document'}.${format}`;
}

export async function exportDocument(content: string, path: string, format: ExportFormat): Promise<boolean> {
  const { invoke } = await import('@tauri-apps/api/core');
  // The save dialog runs in the native process; only a single-use token comes back.
  const token = await invoke<string | null>('export_choose_destination', { fileName: exportFileName(path, format), format });
  if (!token) return false;
  const options: ExportOptions = { notePath: path };
  const bytes = format === 'docx'
    ? await (await import('./docx-export')).createDocx(content, options)
    : await (await import('./pdf-export')).createPdf(content, undefined, options);
  await invoke('export_document', bytes, { headers: { 'x-export-token': token } });
  return true;
}
