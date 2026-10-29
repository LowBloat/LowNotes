import { renderMermaidSvg } from './mermaid-renderer';

export interface ExportImage { data: Uint8Array; width: number; height: number }
export interface ExportOptions {
  notePath?: string;
  resolveImage?: (src: string, notePath?: string) => Promise<ExportImage>;
  renderDiagram?: (source: string, language?: string) => Promise<ExportImage>;
  renderEmoji?: (text: string) => Promise<ExportImage>;
}

const MAX_PIXELS = 16_000_000;
let diagramId = 0;

function canvasImage(canvas: HTMLCanvasElement, width: number, height: number): Promise<ExportImage> {
  return new Promise((resolve, reject) => canvas.toBlob(async (blob) => {
    if (!blob) { reject(new Error('export.imageFailed')); return; }
    resolve({ data: new Uint8Array(await blob.arrayBuffer()), width, height });
  }, 'image/png'));
}

async function rasterize(blob: Blob, dimensions?: { width: number; height: number }): Promise<ExportImage> {
  const image = new Image();
  const url = URL.createObjectURL(blob);
  try {
    image.src = url;
    await image.decode();
    const width = dimensions?.width || image.naturalWidth;
    const height = dimensions?.height || image.naturalHeight;
    if (!width || !height) throw new Error('export.imageFailed');
    const scale = Math.min(2, Math.sqrt(MAX_PIXELS / (width * height)));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    const context = canvas.getContext('2d');
    if (!context) throw new Error('export.imageFailed');
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return await canvasImage(canvas, width, height);
  } finally { URL.revokeObjectURL(url); }
}

export async function resolveExportImage(src: string, notePath?: string): Promise<ExportImage> {
  try {
    let blob: Blob;
    if (/^data:image\//i.test(src)) {
      blob = await (await fetch(src)).blob();
    } else {
      const { isTauri, invoke } = await import('@tauri-apps/api/core');
      if (isTauri()) {
        const result = await invoke<{ bytes: number[]; mime_type: string }>('load_export_image', { src, notePath: notePath || '' });
        blob = new Blob([new Uint8Array(result.bytes)], { type: result.mime_type });
      } else {
        const base = new URL(notePath || '.', document.baseURI);
        const response = await fetch(new URL(src, base));
        if (!response.ok) throw new Error('export.imageFailed');
        blob = await response.blob();
      }
    }
    if (blob.size > 20 * 1024 * 1024) throw new Error('export.imageFailed');
    return await rasterize(blob);
  } catch { throw new Error('export.imageFailed'); }
}

export async function renderExportDiagram(source: string, language = 'mermaid'): Promise<ExportImage> {
  if (language === 'excalidraw') return renderExportDrawing(source);
  const host = document.createElement('div');
  host.style.cssText = 'position:absolute;left:-100000px;top:0;width:1200px;background:white;pointer-events:none';
  document.body.append(host);
  try {
    const svg = await renderMermaidSvg(`export-diagram-${diagramId++}`, source, {
      theme: 'default', fontFamily: 'Arial, sans-serif', htmlLabels: false,
      flowchart: { htmlLabels: false, useMaxWidth: false },
      sequence: { wrap: true, noteFontFamily: 'Arial', actorFontFamily: 'Arial', messageFontFamily: 'Arial' },
    }, host);
    const element = new DOMParser().parseFromString(svg, 'image/svg+xml').documentElement;
    const box = element.getAttribute('viewBox')?.split(/[\s,]+/).map(Number);
    const width = box?.[2] || Number.parseFloat(element.getAttribute('width') || '800');
    const height = box?.[3] || Number.parseFloat(element.getAttribute('height') || '400');
    element.setAttribute('width', String(width));
    element.setAttribute('height', String(height));
    element.setAttribute('style', 'background:white');
    return await rasterize(new Blob([new XMLSerializer().serializeToString(element)], { type: 'image/svg+xml' }), { width, height });
  } catch { throw new Error('export.diagramFailed'); }
  finally { host.remove(); }
}

async function renderExportDrawing(source: string): Promise<ExportImage> {
  try {
    const { renderDrawingCanvas } = await import('./excalidraw');
    const rendered = await renderDrawingCanvas(source, false);
    if (!rendered) throw new Error('export.diagramFailed');
    return await canvasImage(rendered.canvas, rendered.width, rendered.height);
  } catch { throw new Error('export.diagramFailed'); }
}

export async function renderExportEmoji(text: string): Promise<ExportImage> {
  const canvas = document.createElement('canvas');
  canvas.width = 64; canvas.height = 64;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('export.imageFailed');
  context.font = '48px "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif';
  context.textAlign = 'center'; context.textBaseline = 'middle';
  context.fillText(text, 32, 34);
  return canvasImage(canvas, 16, 16);
}
