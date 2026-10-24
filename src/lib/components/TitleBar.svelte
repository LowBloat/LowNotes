<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/i18n';
  import { IS_MAC } from '$lib/menu';
  import { closeContextMenu } from '$lib/context-menu.svelte';
  import type { AppMenu } from '$lib/app-menu';
  import type { createWindowFrame } from '$lib/state/window-frame.svelte';
  import MenuList from './MenuList.svelte';

  let { menus, title, detail = '', frame } = $props<{
    menus: AppMenu[];
    title: string;
    /** Shown dimmed after the title, such as the vault name. */
    detail?: string;
    frame: ReturnType<typeof createWindowFrame>;
  }>();

  let buttons = $state<HTMLButtonElement[]>([]);
  let openIndex = $state<number | null>(null);
  // Reopening the same menu from the keyboard remounts it to focus its first entry.
  let openCount = $state(0);
  let focusFirst = $state(false);
  let anchor = $state({ x: 0, y: 0 });
  // Alt pressed and released alone moves the focus to the menu bar, as on Windows.
  let altAlone = false;
  let focusBeforeMenuBar: HTMLElement | null = null;

  function modalOpen() {
    return !!document.querySelector('[data-modal-backdrop]');
  }

  function open(index: number, keyboard = false) {
    if (modalOpen()) return;
    closeContextMenu();
    rememberFocus();
    const rect = buttons[index].getBoundingClientRect();
    anchor = { x: rect.left, y: rect.bottom + 2 };
    focusFirst = keyboard;
    openIndex = index;
    openCount += 1;
    buttons[index].focus();
  }

  // Closing a menu returns the focus to where it was before the menu bar.
  function close(restoreFocus: boolean) {
    const index = openIndex;
    openIndex = null;
    if (restoreFocus && index !== null) leaveMenuBar(buttons[index]);
  }

  function rememberFocus() {
    const active = document.activeElement;
    if (active instanceof HTMLElement && !buttons.includes(active as HTMLButtonElement)) focusBeforeMenuBar = active;
  }

  function leaveMenuBar(fallback?: HTMLElement) {
    const target = focusBeforeMenuBar?.isConnected ? focusBeforeMenuBar : fallback;
    focusBeforeMenuBar = null;
    target?.focus();
  }

  function sideways(direction: -1 | 1) {
    if (openIndex === null) return;
    open((openIndex + direction + menus.length) % menus.length, true);
  }

  function handleButtonKeydown(event: KeyboardEvent, index: number) {
    const move = (step: number) => {
      const next = (index + step + menus.length) % menus.length;
      if (openIndex !== null) open(next, true);
      else buttons[next]?.focus();
    };
    const keys: Record<string, () => void> = {
      ArrowRight: () => move(1),
      ArrowLeft: () => move(-1),
      ArrowDown: () => open(index, true),
      Enter: () => open(index, true),
      ' ': () => open(index, true),
      Escape: () => (openIndex !== null ? close(true) : leaveMenuBar()),
    };
    const handler = keys[event.key];
    if (!handler) return;
    event.preventDefault();
    handler();
  }

  function handleWindowKeydown(event: KeyboardEvent) {
    altAlone = event.key === 'Alt' && !event.repeat && !event.ctrlKey && !event.shiftKey && !event.metaKey;
    if (event.key === 'F10' && !event.shiftKey && !event.ctrlKey && !event.altKey && !IS_MAC && !modalOpen()) {
      event.preventDefault();
      focusMenuBar();
    }
  }

  function handleWindowKeyup(event: KeyboardEvent) {
    if (event.key !== 'Alt' || !altAlone || IS_MAC || modalOpen()) return;
    altAlone = false;
    event.preventDefault();
    if (buttons.includes(document.activeElement as HTMLButtonElement)) {
      close(false);
      leaveMenuBar();
    } else focusMenuBar();
  }

  function focusMenuBar() {
    rememberFocus();
    buttons[0]?.focus();
  }

  function handlePointerDown(event: PointerEvent) {
    const target = event.target as Element;
    if (openIndex !== null && !target.closest?.('[data-menu], [data-menubar]')) close(false);
  }

  onMount(() => {
    void frame.start();
    return frame.stop;
  });
</script>

<svelte:window onkeydown={handleWindowKeydown} onkeyup={handleWindowKeyup} onpointerdown={handlePointerDown} onblur={() => close(false)} />

<header
  class="titlebar relative z-[80] flex shrink-0 items-stretch border-b border-[var(--border)] bg-[var(--bg-sidebar)] text-xs select-none {frame.state.focused ? 'text-[var(--text-muted)]' : 'text-[var(--text-dim)]'}"
  class:mac={IS_MAC}
  data-tauri-drag-region
>
  {#if !IS_MAC}
    <img src="/favicon.png" alt="" class="w-4 h-4 self-center mx-3" draggable="false" data-tauri-drag-region />
    <div role="menubar" aria-label={$t('titlebar.menubar')} data-menubar class="flex items-stretch">
      {#each menus as menu, index (menu.id)}
        <button
          bind:this={buttons[index]}
          type="button"
          role="menuitem"
          aria-haspopup="menu"
          aria-expanded={openIndex === index}
          tabindex={index === 0 ? 0 : -1}
          onclick={() => (openIndex === index ? close(true) : open(index))}
          onpointerenter={() => { if (openIndex !== null && openIndex !== index) open(index); }}
          onkeydown={(event) => handleButtonKeydown(event, index)}
          class="px-2.5 rounded-md my-1 outline-none hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)] focus-visible:bg-[var(--bg-hover)] focus-visible:text-[var(--text-main)] {openIndex === index ? 'bg-[var(--bg-active)] text-[var(--accent-light)]' : ''}"
        >{menu.label}</button>
      {/each}
    </div>
  {/if}

  <!-- Centered on the window, not on the space between menus and buttons. -->
  <div class="pointer-events-none absolute inset-y-0 left-1/2 -translate-x-1/2 flex items-center gap-1.5 max-w-[45%] min-w-0">
    <span class="truncate font-medium {frame.state.focused ? 'text-[var(--text-main)]' : ''}">{title}</span>
    {#if detail}<span class="truncate text-[var(--text-dim)]">— {detail}</span>{/if}
  </div>

  <div class="flex-1" data-tauri-drag-region></div>

  {#if !IS_MAC}
    <div class="flex items-stretch">
      <button type="button" class="window-button" onclick={frame.minimize} aria-label={$t('titlebar.minimize')} title={$t('titlebar.minimize')}>
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" /></svg>
      </button>
      <button type="button" class="window-button" onclick={frame.toggleMaximize}
        aria-label={frame.state.maximized ? $t('titlebar.restore') : $t('titlebar.maximize')} title={frame.state.maximized ? $t('titlebar.restore') : $t('titlebar.maximize')}>
        {#if frame.state.maximized}
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" /></svg>
        {:else}
          <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5h9v9h-9z" /></svg>
        {/if}
      </button>
      <button type="button" class="window-button close" onclick={frame.close} aria-label={$t('titlebar.close')} title={$t('titlebar.close')}>
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" /></svg>
      </button>
    </div>
  {/if}
</header>

{#if openIndex !== null}
  {#key openCount}
    <MenuList entries={menus[openIndex].entries} x={anchor.x} y={anchor.y} label={menus[openIndex].label} {focusFirst} onClose={close} onSideways={sideways} />
  {/key}
{/if}

<style>
  .titlebar { height: var(--titlebar-height); min-height: var(--titlebar-height); }
  /* Room for the native traffic lights drawn over the bar. */
  .titlebar.mac { padding-left: 78px; }
  .window-button { width: 46px; display: grid; place-items: center; color: var(--text-muted); outline: none; }
  .window-button:hover, .window-button:focus-visible { background: var(--bg-hover); color: var(--text-main); }
  .window-button.close:hover, .window-button.close:focus-visible { background: #c42b1c; color: #fff; }
  .window-button svg { width: 10px; height: 10px; fill: none; stroke: currentColor; stroke-width: 1; shape-rendering: crispEdges; }
</style>
