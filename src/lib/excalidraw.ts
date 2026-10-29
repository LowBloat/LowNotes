import type { BinaryFileData, ExcalidrawImperativeAPI } from '@excalidraw/excalidraw/types';
import type { ExcalidrawElement } from '@excalidraw/excalidraw/element/types';
import { emptyDrawing, parseDrawing, serializeDrawing, type DrawingElement, type DrawingScene } from './drawing-blocks';

/** Excalidraw's fonts are served by the app (see vite.config.js), never from its CDN. */
const ASSET_PATH = '/excalidraw/';
/** Local edits reach the note once the pen has rested this long. */
const SAVE_DELAY_MS = 300;
/** Rendered drawings keep about twice the screen resolution, within this many pixels per side. */
const MAX_IMAGE_SIDE = 4096;

async function load() {
  (window as { EXCALIDRAW_ASSET_PATH?: string }).EXCALIDRAW_ASSET_PATH = ASSET_PATH;
  // Xiaolai, the CJK fallback, is not shipped (13 MB). Without its faces in the
  // document, that text uses system fonts instead of a blocked request to the CDN.
  const fonts = document.fonts;
  const add = fonts.add.bind(fonts);
  fonts.add = (face: FontFace) => face.family.replace(/["']/g, '') === 'Xiaolai' ? fonts : add(face);
  const [lib, React, ReactDOM] = await Promise.all([
    import('@excalidraw/excalidraw'), import('react'), import('react-dom/client'), import('@excalidraw/excalidraw/index.css'),
  ]);
  return { lib, React, ReactDOM };
}

let runtime: ReturnType<typeof load> | null = null;

/** Excalidraw and React are large: they load the first time a drawing is shown. */
function loadExcalidraw() {
  runtime ??= load().catch((error) => { runtime = null; throw error; });
  return runtime;
}

type Library = Awaited<ReturnType<typeof load>>['lib'];

function restoreScene(lib: Library, scene: DrawingScene) {
  return lib.restore({ elements: scene.elements as unknown as ExcalidrawElement[], appState: scene.appState, files: scene.files as never }, null, null);
}

/** The drawing as a canvas for the preview and exports, or null when it has nothing to show.
 * `width` and `height` are its size in CSS pixels. A canvas rather than an SVG: inlining
 * fonts into an SVG compiles code at runtime, which the app's CSP forbids. */
export async function renderDrawingCanvas(content: string, dark: boolean): Promise<{ canvas: HTMLCanvasElement; width: number; height: number } | null> {
  const scene = parseDrawing(content);
  if (!scene) throw new Error('drawing.invalid');
  if (!scene.elements.some(element => !element.isDeleted)) return null;
  const { lib } = await loadExcalidraw();
  const data = restoreScene(lib, scene);
  let size = { width: 0, height: 0 };
  const canvas = await lib.exportToCanvas({
    elements: data.elements.filter(element => !element.isDeleted),
    appState: { ...data.appState, exportBackground: true, exportWithDarkMode: dark },
    files: data.files,
    exportPadding: 16,
    getDimensions: (width: number, height: number) => {
      size = { width, height };
      const scale = Math.min(2, MAX_IMAGE_SIDE / Math.max(width, height, 1));
      return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)), scale };
    },
  });
  return { canvas, ...size };
}

export interface DrawingEditorHandle {
  /** Merges a version of the drawing changed elsewhere (another device) into the canvas. */
  merge(content: string): void;
  /** Sends pending changes right away. */
  flush(): void;
  destroy(): void;
}

export async function mountDrawingEditor(host: HTMLElement, options: {
  content: string;
  theme: 'light' | 'dark';
  langCode: string;
  onChange: (content: string) => void;
  onOpenLink: (url: string) => void;
}): Promise<DrawingEditorHandle> {
  const { lib, React, ReactDOM } = await loadExcalidraw();
  const scene = parseDrawing(options.content) ?? emptyDrawing();
  let api: ExcalidrawImperativeAPI | null = null;
  let saved = serializeDrawing(scene);
  let version = -1;
  let background = '';
  let timer: ReturnType<typeof setTimeout> | null = null;

  const save = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    if (!api) return;
    const content = serializeDrawing({
      elements: api.getSceneElementsIncludingDeleted() as unknown as DrawingElement[],
      appState: { viewBackgroundColor: api.getAppState().viewBackgroundColor },
      files: api.getFiles() as unknown as Record<string, unknown>,
    });
    if (content === saved) return;
    saved = content;
    options.onChange(content);
  };

  const { createElement: h } = React;
  const { MainMenu, WelcomeScreen } = lib;
  const root = ReactDOM.createRoot(host);
  root.render(h(lib.Excalidraw, {
    initialData: { ...restoreScene(lib, scene), scrollToContent: true },
    excalidrawAPI: (value: ExcalidrawImperativeAPI) => { api = value; },
    theme: options.theme,
    langCode: options.langCode,
    autoFocus: true,
    // Pictures would be stored inside the note as base64; they stay in Markdown instead.
    UIOptions: { canvasActions: { loadScene: false, saveToActiveFile: false, export: false }, tools: { image: false } },
    onPaste: (data: { files?: unknown }, event: ClipboardEvent | null) =>
      ![...(event?.clipboardData?.items ?? [])].some(item => item.kind === 'file'),
    onLinkOpen: (element: { link: string | null }, event: CustomEvent) => {
      event.preventDefault();
      if (element.link && /^https?:\/\//i.test(element.link)) options.onOpenLink(element.link);
    },
    // Scrolling and selection also call onChange: only element or background changes are saved.
    onChange: (elements: readonly ExcalidrawElement[], appState: { viewBackgroundColor: string }) => {
      const next = lib.getSceneVersion(elements);
      if (next === version && appState.viewBackgroundColor === background) return;
      version = next;
      background = appState.viewBackgroundColor;
      if (timer) clearTimeout(timer);
      timer = setTimeout(save, SAVE_DELAY_MS);
    },
  },
  h(MainMenu, null,
    h(MainMenu.DefaultItems.SaveAsImage),
    h(MainMenu.DefaultItems.SearchMenu),
    h(MainMenu.DefaultItems.Help),
    h(MainMenu.DefaultItems.ClearCanvas),
    h(MainMenu.Separator),
    h(MainMenu.DefaultItems.ChangeCanvasBackground)),
  h(WelcomeScreen, null,
    h(WelcomeScreen.Hints.MenuHint),
    h(WelcomeScreen.Hints.ToolbarHint),
    h(WelcomeScreen.Hints.HelpHint))));

  return {
    merge(content) {
      const remote = parseDrawing(content);
      if (!api || !remote) return;
      // Pending local strokes are still in the scene: the reconciled result is saved after this.
      const data = restoreScene(lib, remote);
      const files = Object.values(data.files) as BinaryFileData[];
      if (files.length) api.addFiles(files);
      api.updateScene({
        elements: lib.reconcileElements(api.getSceneElementsIncludingDeleted(), data.elements as never, api.getAppState()),
        appState: { viewBackgroundColor: data.appState.viewBackgroundColor },
        captureUpdate: lib.CaptureUpdateAction.NEVER,
      });
      saved = content;
    },
    flush: save,
    destroy() {
      save();
      api = null;
      root.unmount();
    },
  };
}
