import { parseDocument, type DocumentBlock, type TextSpan } from './document-model';
import { resolveExportImage, renderExportDiagram, renderExportEmoji, type ExportImage, type ExportOptions } from './export-assets';

export type PreparedSpan = TextSpan & { asset?: ExportImage };
export type PreparedBlock = Omit<DocumentBlock, 'spans' | 'rows'> & {
  spans: PreparedSpan[];
  rows?: PreparedSpan[][][];
  asset?: ExportImage;
};

export async function prepareDocument(content: string, options: ExportOptions): Promise<PreparedBlock[]> {
  const images = new Map<string, Promise<ExportImage>>();
  const diagrams = new Map<string, Promise<ExportImage>>();
  const emojis = new Map<string, Promise<ExportImage>>();
  async function span(source: TextSpan): Promise<PreparedSpan> {
    if (source.image) {
      const src = source.image.src;
      if (!images.has(src)) images.set(src, (options.resolveImage || resolveExportImage)(src, options.notePath));
      return { ...source, asset: await images.get(src)! };
    }
    if (source.emoji && (typeof document !== 'undefined' || options.renderEmoji)) {
      if (!emojis.has(source.text)) emojis.set(source.text, (options.renderEmoji || renderExportEmoji)(source.text));
      return { ...source, asset: await emojis.get(source.text)! };
    }
    return source;
  }
  const result: PreparedBlock[] = [];
  for (const block of parseDocument(content)) {
    const prepared: PreparedBlock = { ...block, spans: await Promise.all(block.spans.map(span)),
      rows: block.rows ? await Promise.all(block.rows.map(async (row) => Promise.all(row.map(async (cell) => Promise.all(cell.map(span)))))) : undefined };
    if (block.kind === 'diagram') {
      const code = block.source || '', key = `${block.language}:${code}`;
      if (!diagrams.has(key)) diagrams.set(key, (options.renderDiagram || renderExportDiagram)(code, block.language));
      prepared.asset = await diagrams.get(key)!;
    }
    result.push(prepared);
  }
  return result;
}

export function columnFractions(rows: TextSpan[][][]): number[] {
  const count = rows[0]?.length || 1;
  const weights = Array.from({ length: count }, (_, column) => Math.sqrt(Math.min(120,
    Math.max(8, ...rows.map((row) => row[column]?.reduce((sum, span) => sum + (span.image ? 20 : span.text.length), 0) || 0)))));
  const total = weights.reduce((sum, weight) => sum + weight, 0);
  return weights.map((weight) => weight / total);
}
