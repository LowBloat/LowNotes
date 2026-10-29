import type { ParagraphChild, ILevelsOptions } from 'docx';
import { prepareDocument, columnFractions, type PreparedSpan, type PreparedBlock } from './prepared-document';
import type { ExportImage, ExportOptions } from './export-assets';

const PAGE_WIDTH = 9638;
const COLORS: Record<string, string> = { info: 'EFF6FF', tip: 'F0FDF4', warning: 'FFFBEB', danger: 'FEF2F2' };

export async function createDocx(content: string, options: ExportOptions = {}): Promise<Uint8Array> {
  const { Document, Packer, Paragraph, TextRun, ImageRun, CheckBox, FootnoteReferenceRun,
    ExternalHyperlink, Table, TableRow, TableCell, WidthType, HeadingLevel, BorderStyle,
    AlignmentType, LevelFormat, TableLayoutType } = await import('docx');
  const blocks = await prepareDocument(content, options);
  const headings = [HeadingLevel.HEADING_1, HeadingLevel.HEADING_2, HeadingLevel.HEADING_3,
    HeadingLevel.HEADING_4, HeadingLevel.HEADING_5, HeadingLevel.HEADING_6];
  function imageRun(asset: ExportImage, available: number, description: string, emoji = false) {
    const scale = emoji ? 15 / asset.height : Math.min(1, available / 15 / asset.width, 900 / asset.height);
    return new ImageRun({ type: 'png', data: asset.data,
      transformation: { width: asset.width * scale, height: asset.height * scale },
      altText: { name: description || 'Image', description, title: description },
    });
  }
  function runs(spans: PreparedSpan[], available = PAGE_WIDTH): ParagraphChild[] {
    return spans.flatMap<ParagraphChild>((span) => {
      if (span.asset) {
        const run = imageRun(span.asset, available, span.image?.alt || span.text, span.emoji);
        return span.url && /^(https?:|mailto:)/i.test(span.url)
          ? [new ExternalHyperlink({ link: span.url, children: [run] })] : [run];
      }
      if (span.checkbox !== undefined) return [new CheckBox({ checked: span.checkbox,
        checkedState: { value: '2611', font: 'Segoe UI Symbol' }, uncheckedState: { value: '2610', font: 'Segoe UI Symbol' } })];
      if (span.footnoteId) return [new FootnoteReferenceRun(span.footnoteId)];
      const textRuns = span.text.split('\n').map((text, index) => new TextRun({ text, break: index ? 1 : undefined,
        bold: span.bold, italics: span.italic, strike: span.strike,
        underline: span.underline || span.url ? {} : undefined,
        highlight: span.highlight ? 'yellow' : undefined,
        subScript: span.subscript, superScript: span.superscript,
        font: span.code ? 'Consolas' : undefined, shading: span.code ? { fill: 'F3F4F6' } : undefined,
        ...(span.url ? { color: '1565C0' } : {}),
      }));
      return span.url && /^(https?:|mailto:)/i.test(span.url)
        ? [new ExternalHyperlink({ link: span.url, children: textRuns })] : textRuns;
    });
  }
  const numbering = new Map<number, { reference: string; levels: ILevelsOptions[] }>();
  function paragraph(block: PreparedBlock) {
    const isCode = block.kind === 'code';
    const codeLines = block.source?.split('\n') || [];
    const shortCode = isCode && codeLines.length <= 20 && codeLines.every((line) => line.length <= 120);
    const hasCheckbox = block.spans.some((span) => span.checkbox !== undefined);
    const list = block.list;
    if (list && !numbering.has(list.id)) numbering.set(list.id, {
      reference: `list-${list.id}`,
      levels: Array.from({ length: 9 }, (_, level) => ({ level,
        format: list.ordered ? LevelFormat.DECIMAL : LevelFormat.BULLET,
        text: list.ordered ? `%${level + 1}.` : '•', start: list.start,
        alignment: AlignmentType.LEFT,
        style: { paragraph: { indent: { left: (level + 1) * 360, hanging: 280 } }, run: { font: 'Calibri' } },
      })),
    });
    const numbered = list?.first && (!hasCheckbox || list.ordered);
    const left = (block.indent || (block.quote || block.callout ? 1 : 0)) * 360;
    return new Paragraph({
      children: block.asset ? [imageRun(block.asset, PAGE_WIDTH - left, block.language === 'excalidraw' ? 'Drawing' : 'Mermaid diagram')]
        : runs(block.spans.map((span) => ({ ...span, bold: block.definitionTerm || span.bold })), PAGE_WIDTH - left),
      heading: block.kind === 'heading' ? headings[(block.level || 1) - 1] : undefined,
      numbering: numbered ? { reference: `list-${list!.id}`, level: Math.min(list!.level, 8) } : undefined,
      keepNext: block.kind === 'heading', keepLines: block.kind === 'diagram' || shortCode, widowControl: true,
      alignment: block.asset || (block.spans.length === 1 && block.spans[0].image) ? AlignmentType.CENTER : undefined,
      spacing: { after: block.kind === 'heading' ? 180 : 140, before: block.kind === 'heading' ? 260 : 0,
        line: isCode ? 260 : 300 },
      indent: left && !numbered ? { left } : undefined,
      shading: isCode ? { fill: 'F3F4F6' } : block.callout ? { fill: COLORS[block.callout] || COLORS.info } : undefined,
      border: block.kind === 'rule' ? { bottom: { style: BorderStyle.SINGLE, size: 6, color: 'CBD5E1' } }
        : block.quote || block.callout ? { left: { style: BorderStyle.SINGLE, size: 16, color: block.quote ? '94A3B8' : '60A5FA', space: 8 } } : undefined,
      ...(isCode ? { style: 'CodeBlock' } : {}),
    });
  }
  function table(block: PreparedBlock) {
    const widths = columnFractions(block.rows || []).map((fraction) => Math.floor(PAGE_WIDTH * fraction));
    return new Table({ width: { size: PAGE_WIDTH, type: WidthType.DXA }, columnWidths: widths,
      layout: TableLayoutType.FIXED,
      rows: (block.rows || []).map((row, index) => new TableRow({ tableHeader: index === 0,
        children: row.map((cell, column) => new TableCell({ width: { size: widths[column], type: WidthType.DXA },
          shading: index === 0 ? { fill: 'EAF0F6' } : index % 2 === 0 ? { fill: 'F8FAFC' } : undefined,
          margins: { top: 120, bottom: 120, left: 140, right: 140 },
          children: [new Paragraph({ alignment: block.alignments?.[column] || 'left',
            spacing: { after: 0, line: 280 },
            children: runs(cell.map((span) => ({ ...span, bold: index === 0 || span.bold })), widths[column] - 280),
          })],
        })),
      })),
    });
  }
  const footnotes: Record<string, { children: InstanceType<typeof Paragraph>[] }> = {};
  const children: Array<InstanceType<typeof Paragraph> | InstanceType<typeof Table>> = [];
  for (const block of blocks) {
    if (block.footnoteId) (footnotes[block.footnoteId] ||= { children: [] }).children.push(paragraph(block));
    else children.push(block.kind === 'table' ? table(block) : paragraph(block));
  }
  const document = new Document({ creator: 'LowNotes', footnotes,
    numbering: { config: Array.from(numbering.values()) },
    styles: {
      default: {
        document: { run: { font: 'Calibri', size: 22, color: '202B3B' }, paragraph: { spacing: { line: 300 } } },
        heading1: { run: { color: '111827', bold: true, size: 40 } },
        heading2: { run: { color: '111827', bold: true, size: 32 } },
        heading3: { run: { color: '111827', bold: true, size: 28 } },
        heading4: { run: { color: '111827', bold: true, size: 24 } },
        heading5: { run: { color: '111827', bold: true, size: 22 } },
        heading6: { run: { color: '111827', bold: true, size: 22 } },
      },
      paragraphStyles: [{ id: 'CodeBlock', name: 'Code block', basedOn: 'Normal',
        run: { font: 'Consolas', size: 19 }, paragraph: { spacing: { line: 260 } } }],
    },
    sections: [{ properties: { page: { size: { width: 11906, height: 16838 },
      margin: { top: 1134, bottom: 1134, left: 1134, right: 1134 } } }, children }],
  });
  return new Uint8Array(await (await Packer.toBlob(document)).arrayBuffer());
}
