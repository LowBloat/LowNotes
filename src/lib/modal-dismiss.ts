/** Shared behavior for app modals: the last mounted modal owns Escape, Tab
 * stays inside its dialog, focus moves in on open and returns on close, and a
 * dialog without an accessible name is labelled by its first heading. */
const FOCUSABLE = 'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';
let labelSequence = 0;

function focusable(container: HTMLElement): HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((element) => element.getClientRects().length > 0);
}

function labelDialog(dialog: HTMLElement) {
  if (dialog.hasAttribute('aria-labelledby') || dialog.hasAttribute('aria-label')) return;
  const heading = dialog.querySelector<HTMLElement>('h1, h2, h3');
  if (!heading) return;
  if (!heading.id) heading.id = `modal-title-${++labelSequence}`;
  dialog.setAttribute('aria-labelledby', heading.id);
}

export function dismissibleModal(node: HTMLElement, onDismiss: () => void) {
  node.dataset.modalBackdrop = '';
  let pressedOutside = false;
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const dialog = node.querySelector<HTMLElement>('[role="dialog"], [role="alertdialog"]') ?? node;

  function isTopmost() {
    const overlays = document.querySelectorAll('[data-modal-backdrop]');
    return overlays[overlays.length - 1] === node;
  }

  function onPointerDown(event: PointerEvent) {
    pressedOutside = event.button === 0 && event.target === node;
  }

  function onPointerUp(event: PointerEvent) {
    if (pressedOutside && event.target === node && isTopmost()) onDismiss();
    pressedOutside = false;
  }

  function onKeyDown(event: KeyboardEvent) {
    if (!isTopmost()) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopImmediatePropagation();
      onDismiss();
      return;
    }
    if (event.key !== 'Tab') return;
    const elements = focusable(dialog);
    if (elements.length === 0) {
      event.preventDefault();
      dialog.focus();
      return;
    }
    const first = elements[0];
    const last = elements[elements.length - 1];
    const active = document.activeElement;
    const outside = !(active instanceof Node) || !dialog.contains(active);
    if (event.shiftKey && (outside || active === first || active === dialog)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (outside || active === last || active === dialog)) {
      event.preventDefault();
      first.focus();
    }
  }

  labelDialog(dialog);
  // Move focus in once the dialog has rendered. Prefer a text field; otherwise
  // focus the dialog itself so its title is announced and no action is armed.
  queueMicrotask(() => {
    if (!node.isConnected || dialog.contains(document.activeElement)) return;
    const field = dialog.querySelector<HTMLElement>('input:not([type="hidden"]):not(:disabled):not([readonly]), textarea:not(:disabled):not([readonly])');
    if (field && field.getClientRects().length) {
      field.focus();
      return;
    }
    if (!dialog.hasAttribute('tabindex')) dialog.setAttribute('tabindex', '-1');
    dialog.focus();
  });

  node.addEventListener('pointerdown', onPointerDown);
  node.addEventListener('pointerup', onPointerUp);
  window.addEventListener('keydown', onKeyDown, true);

  return {
    destroy() {
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointerup', onPointerUp);
      window.removeEventListener('keydown', onKeyDown, true);
      if (previous && previous.isConnected) previous.focus();
    },
  };
}
