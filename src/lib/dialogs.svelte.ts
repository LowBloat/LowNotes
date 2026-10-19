/** In-app replacements for window.alert/confirm/prompt. Native dialogs block
 * the WebView, ignore the theme and translations, and `prompt` is unreliable
 * in WKWebView (macOS). Requests are queued and shown one at a time by
 * `AppDialog.svelte`, mounted once by the page. */

export type DialogKind = 'alert' | 'confirm' | 'prompt';

export interface DialogRequest {
  kind: DialogKind;
  message: string;
  /** Initial text for prompts. */
  value?: string;
  /** Destructive confirmation: styled as danger and Cancel gets the focus. */
  danger?: boolean;
  confirmLabel?: string;
  resolve: (value: unknown) => void;
}

export const dialogState = $state<{ current: DialogRequest | null; queue: DialogRequest[] }>({ current: null, queue: [] });

function enqueue(request: DialogRequest) {
  if (dialogState.current) dialogState.queue.push(request);
  else dialogState.current = request;
}

/** Resolves the visible request and shows the next one. */
export function settleDialog(value: unknown) {
  const request = dialogState.current;
  dialogState.current = dialogState.queue.shift() ?? null;
  request?.resolve(value);
}

/** Value a request resolves with when dismissed (Escape, backdrop, Cancel). */
export function dismissValue(kind: DialogKind): unknown {
  return kind === 'confirm' ? false : kind === 'prompt' ? null : undefined;
}

export function appAlert(message: string): Promise<void> {
  return new Promise((resolve) => enqueue({ kind: 'alert', message, resolve: () => resolve() }));
}

export function appConfirm(message: string, options: { danger?: boolean; confirmLabel?: string } = {}): Promise<boolean> {
  return new Promise((resolve) => enqueue({ kind: 'confirm', message, ...options, resolve: (value) => resolve(value === true) }));
}

export function appPrompt(message: string, value = ''): Promise<string | null> {
  return new Promise((resolve) => enqueue({ kind: 'prompt', message, value, resolve: (result) => resolve(typeof result === 'string' ? result : null) }));
}
