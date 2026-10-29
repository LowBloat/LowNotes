import { expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { inflateRawSync } from 'node:zlib';
import { createDocx, createPdf, exportFileName, parseDocument, type PdfFonts } from '../src/lib/document-export';
import { renderChatMarkdown } from '../src/lib/markdown';

const sample = '# Plano de estudos de Python\n\nAprenda **funções**, *coleções* e organização com prática.\n\n## Acompanhamento\n\n- [ ] Estudar variáveis\n- [x] Instalar Python\n\n3. Ler documentação\n4. Praticar\n   - Revisar exemplos\n\n| Semana | Objetivo |\n| --- | --- |\n| 1 | Variáveis e operações |\n| 2 | Funções e coleções |\n\n```python\nprint("Olá, ação!")\n```\n\n[Documentação](https://docs.python.org/3/)\n';

export async function testPdfFonts(): Promise<PdfFonts> {
  const entries = await Promise.all(Object.entries({ normal: 'Regular', bold: 'Bold', italic: 'Italic', bolditalic: 'BoldItalic' }).map(async ([key, style]) =>
    [key, (await readFile(new URL(`../static/fonts/NotoSans-${style}.ttf`, import.meta.url))).toString('base64')]
  ));
  return { ...Object.fromEntries(entries), mono: (await readFile(new URL('../static/fonts/NotoSansMono-Regular.ttf', import.meta.url))).toString('base64') } as unknown as PdfFonts;
}

// Read generated ZIP members without adding a test-only runtime dependency.
function unzipText(bytes: Uint8Array, name: string): string {
  const data = Buffer.from(bytes);
  let cursor = 0;
  while ((cursor = data.indexOf(Buffer.from([0x50, 0x4b, 0x01, 0x02]), cursor)) >= 0) {
    const method = data.readUInt16LE(cursor + 10);
    const size = data.readUInt32LE(cursor + 20);
    const nameLength = data.readUInt16LE(cursor + 28);
    const extraLength = data.readUInt16LE(cursor + 30);
    const commentLength = data.readUInt16LE(cursor + 32);
    const localOffset = data.readUInt32LE(cursor + 42);
    const memberName = data.subarray(cursor + 46, cursor + 46 + nameLength).toString();
    if (memberName === name) {
      const start = localOffset + 30 + data.readUInt16LE(localOffset + 26) + data.readUInt16LE(localOffset + 28);
      const member = data.subarray(start, start + size);
      return (method === 8 ? inflateRawSync(member) : member).toString();
    }
    cursor += 46 + nameLength + extraLength + commentLength;
  }
  throw new Error(`Missing ZIP entry ${name}`);
}

test('export preserves tasks, nested lists, ordered starts, table cells and accented code', () => {
  const blocks = parseDocument(sample);
  const text = blocks.flatMap((b) => b.spans).map((s) => s.text).join('\n');
  expect(text).toContain('Estudar variáveis');
  expect(blocks.flatMap((b) => b.spans).filter((s) => s.checkbox !== undefined).map((s) => s.checkbox)).toEqual([false, true]);
  expect(blocks.find((b) => b.list?.ordered)?.list?.start).toBe(3);
  expect(blocks.some((b) => b.indent === 2)).toBe(true);
  expect(blocks.find((b) => b.kind === 'table')?.rows?.length).toBe(3);
  expect(blocks.find((b) => b.kind === 'code')?.spans[0].text).toContain('Olá, ação!');
});

test('Word contains paragraphs, formatted runs, tables and actual external hyperlinks', async () => {
  const bytes = await createDocx(sample);
  const xml = unzipText(bytes, 'word/document.xml');
  expect(xml).toContain('Plano de estudos de Python');
  expect(xml).toContain('funções');
  expect(xml).toContain('<w:tbl>');
  expect(xml).toContain('<w:b/>');
  expect(xml).toContain('<w:i/>');
  expect(xml).toContain('Estudar variáveis');
  expect(xml).toContain('<w14:checked w14:val="0"/>');
  expect(xml).toContain('<w14:checked w14:val="1"/>');
  expect(unzipText(bytes, 'word/numbering.xml')).toContain('<w:start w:val="3"/>');
  expect(unzipText(bytes, 'word/_rels/document.xml.rels')).toContain('https://docs.python.org/3/');
});

test('PDF embeds Unicode fonts, paginates long text and retains link annotations', async () => {
  const bytes = await createPdf(sample + '\n\n' + 'Um parágrafo longo com acentuação e funções. '.repeat(600), await testPdfFonts());
  const data = Buffer.from(bytes).toString('latin1');
  expect(data.startsWith('%PDF-')).toBe(true);
  expect(data.match(/\/Type \/Page\b/g)!.length).toBeGreaterThan(1);
  expect(data).toContain('/FontFile2');
  expect(data).toContain('/ToUnicode');
  expect(data).toContain('/URI (https://docs.python.org/3/)');
});

test('chat escapes raw HTML and dangerous links, while keeping source links and code diagrams visible', () => {
  const rendered = renderChatMarkdown('<img src=x onerror=alert(1)>\n\n[bad](javascript:alert(1))\n\n[Nota](lownotes://open?path=Plano.md&line=2)\n\n```mermaid\ngraph TD; A-->B\n```');
  expect(rendered).not.toContain('<img');
  expect(rendered).not.toContain('href="javascript:');
  expect(rendered).toContain('href="lownotes://');
  expect(rendered).toContain('graph TD');
  expect(rendered).not.toContain('class="mermaid-svg"');
});

test('export filename is derived from the note basename', () => {
  expect(exportFileName('Python/Plano de estudos.md', 'pdf')).toBe('Plano de estudos.pdf');
  expect(exportFileName('notes\\A.markdown', 'docx')).toBe('A.docx');
});

test('exports the preview dialect without raw extension markup', () => {
  const blocks = parseDocument('~~removed~~ ++added++ ==marked== H~2~O X^2^ [[Path|Label]][^note]\n\nTerm\n: Definition\n\n::: warning\nBe careful\n:::\n\n[^note]: Footnote');
  const spans = blocks.flatMap((block) => block.spans);
  expect(spans.find((span) => span.text === 'removed')?.strike).toBe(true);
  expect(spans.find((span) => span.text === 'added')?.underline).toBe(true);
  expect(spans.find((span) => span.text === 'marked')?.highlight).toBe(true);
  expect(spans.some((span) => span.subscript && span.text === '2')).toBe(true);
  expect(spans.some((span) => span.superscript && span.text === '2')).toBe(true);
  expect(spans.some((span) => span.text === 'Label')).toBe(true);
  expect(spans.some((span) => span.footnoteId === 1)).toBe(true);
  expect(blocks.some((block) => block.footnoteId === 1)).toBe(true);
  expect(blocks.some((block) => block.callout === 'warning')).toBe(true);
  expect(blocks.find((block) => block.definitionTerm)?.spans[0].text).toBe('Term');
  expect(parseDocument('first line\nsecond line')[0].spans.map((span) => span.text).join('')).toBe('first line second line');
});

test('Word keeps rich formatting, native footnotes, table alignment and headings', async () => {
  const bytes = await createDocx('# Heading\n\n~~removed~~ ++added++ ==marked== H~2~O X^2^ [^a]\n\n| Left | Right |\n|:---|---:|\n|**bold**|*italic*|\n\n[^a]: A note');
  const xml = unzipText(bytes, 'word/document.xml');
  expect(xml).toContain('<w:strike/>');
  expect(xml).toContain('<w:highlight w:val="yellow"/>');
  expect(xml).toContain('<w:vertAlign w:val="subscript"/>');
  expect(xml).toContain('<w:vertAlign w:val="superscript"/>');
  expect(xml).toContain('<w:footnoteReference w:id="1"/>');
  expect(xml).toContain('<w:jc w:val="right"/>');
  expect(unzipText(bytes, 'word/footnotes.xml')).toContain('A note');
  const heading = xml.slice(xml.indexOf('<w:p>'), xml.indexOf('</w:p>'));
  expect(heading).toContain('Heading1');
  expect(heading).not.toContain('<w:b w:val="false"/>');
});

test('Mermaid and image blocks embed visual assets rather than source text', async () => {
  const png = new Uint8Array(await readFile(new URL('../assets/brand/lownotes_logo.png', import.meta.url)));
  let diagrams = 0, images = 0;
  const options = {
    renderDiagram: async () => { diagrams++; return { data: png, width: 300, height: 300 }; },
    resolveImage: async () => { images++; return { data: png, width: 300, height: 300 }; },
  };
  const content = '```mermaid\ngraph TD\nA-->B\n```\n\n![Logo](logo.png)\n\n![Logo again](logo.png)';
  const docx = await createDocx(content, options);
  const xml = unzipText(docx, 'word/document.xml');
  expect(xml).not.toContain('graph TD');
  expect(xml).not.toContain('logo.png');
  expect(xml.match(/<w:drawing>/g)).toHaveLength(3);
  expect(diagrams).toBe(1); expect(images).toBe(1);
  const pdf = Buffer.from(await createPdf(content, await testPdfFonts(), options)).toString('latin1');
  expect(pdf).toContain('/Subtype /Image');
  expect(diagrams).toBe(2); expect(images).toBe(2);
});

test('Excalidraw drawings export as images rendered by their own language', async () => {
  const png = new Uint8Array(await readFile(new URL('../assets/brand/lownotes_logo.png', import.meta.url)));
  const languages: Array<string | undefined> = [];
  const content = '```excalidraw\n{"elements":[]}\n```\n\n```mermaid\n{"elements":[]}\n```';
  const docx = await createDocx(content, {
    renderDiagram: async (_source: string, language?: string) => { languages.push(language); return { data: png, width: 300, height: 300 }; },
  });
  const xml = unzipText(docx, 'word/document.xml');
  expect(xml).not.toContain('elements');
  expect(xml).toContain('descr="Drawing"');
  expect(xml.match(/<w:drawing>/g)).toHaveLength(2);
  expect(languages).toEqual(['excalidraw', 'mermaid']);
});

test('failed visual assets reject export instead of silently writing code or dropping images', async () => {
  await expect(createDocx('```mermaid\ninvalid diagram\n```', {
    renderDiagram: async () => { throw new Error('export.diagramFailed'); },
  })).rejects.toThrow('export.diagramFailed');
  await expect(createPdf('![Missing](missing.png)', await testPdfFonts(), {
    resolveImage: async () => { throw new Error('export.imageFailed'); },
  })).rejects.toThrow('export.imageFailed');
});

test('PDF paginates rich tables with long cells and wraps long code lines', async () => {
  const content = '| Heading | Result |\n|:---|---:|\n| ' + 'A long cell with **bold** and *italic* text. '.repeat(200)
    + ' CELL_END | 42 |\n\n```python\n' + 'long_identifier_'.repeat(100) + '\n```';
  const bytes = await createPdf(content, await testPdfFonts());
  expect(Buffer.from(bytes).toString('latin1').match(/\/Type \/Page\b/g)!.length).toBeGreaterThan(2);
});

test('Word keeps short code together while allowing long blocks to paginate', async () => {
  const shortCode = '```python\nfirst_line()\nsecond_line()\n```';
  const longCode = '```python\n' + 'another_line()\n'.repeat(80) + '```';
  const xml = unzipText(await createDocx(shortCode + '\n\n' + longCode), 'word/document.xml');
  const paragraphs = xml.match(/<w:p>.*?<\/w:p>/g)!;
  expect(paragraphs.find((paragraph) => paragraph.includes('first_line'))).toContain('<w:keepLines/>');
  expect(paragraphs.find((paragraph) => paragraph.includes('another_line'))).toContain('<w:keepLines w:val="false"/>');
});
