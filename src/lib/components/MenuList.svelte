<script lang="ts">
  import { onMount } from 'svelte';
  import { Check } from 'lucide-svelte';
  import { isSeparator, type MenuEntry, type MenuItem } from '$lib/menu';

  let {
    entries,
    x,
    y,
    label,
    focusFirst = true,
    onClose,
    onSideways,
  } = $props<{
    entries: MenuEntry[];
    /** Viewport position of the top left corner, moved to stay on screen. */
    x: number;
    y: number;
    label?: string;
    /** Focus the first entry on open; a menu bar opened by pointer may not. */
    focusFirst?: boolean;
    /** `restoreFocus` is true when the menu closes without running an entry. */
    onClose: (restoreFocus: boolean) => void;
    /** Left and right arrows, for menu bars that move between menus. */
    onSideways?: (direction: -1 | 1) => void;
  }>();

  let element = $state<HTMLDivElement | null>(null);
  let left = $state(0);
  let top = $state(0);

  onMount(() => {
    left = x;
    top = y;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    left = Math.max(4, Math.min(x, window.innerWidth - rect.width - 4));
    // Open upwards when there is no room below.
    top = y + rect.height > window.innerHeight - 4 ? Math.max(4, y - rect.height) : y;
    if (focusFirst) items()[0]?.focus();
  });

  function items(): HTMLButtonElement[] {
    return element ? [...element.querySelectorAll<HTMLButtonElement>('button[role^="menuitem"]:not(:disabled)')] : [];
  }

  function move(step: number) {
    const all = items();
    if (!all.length) return;
    const index = all.indexOf(document.activeElement as HTMLButtonElement);
    const next = index < 0 ? (step > 0 ? 0 : all.length - 1) : (index + step + all.length) % all.length;
    all[next].focus();
  }

  function handleKeydown(event: KeyboardEvent) {
    const keys: Record<string, () => void> = {
      ArrowDown: () => move(1),
      ArrowUp: () => move(-1),
      Home: () => items()[0]?.focus(),
      End: () => items().at(-1)?.focus(),
      Escape: () => onClose(true),
      Tab: () => onClose(true),
      ArrowLeft: () => onSideways?.(-1),
      ArrowRight: () => onSideways?.(1),
    };
    const handler = keys[event.key];
    if (!handler) {
      // Typing a letter moves to the next entry starting with it.
      if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
        const letter = event.key.toLocaleLowerCase();
        const all = items();
        const start = all.indexOf(document.activeElement as HTMLButtonElement);
        const match = [...all.slice(start + 1), ...all.slice(0, start + 1)]
          .find((button) => button.textContent?.trim().toLocaleLowerCase().startsWith(letter));
        match?.focus();
      }
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    handler();
  }

  function run(item: MenuItem) {
    onClose(true);
    void Promise.resolve().then(item.action);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  bind:this={element}
  role="menu"
  aria-label={label}
  tabindex="-1"
  data-menu
  onkeydown={handleKeydown}
  oncontextmenu={(event) => event.preventDefault()}
  class="fixed z-[70] min-w-[200px] max-w-[320px] max-h-[calc(100vh-8px)] overflow-y-auto rounded-lg border border-[var(--border)] bg-[var(--bg-card)] py-1 shadow-xl text-xs text-[var(--text-main)] select-none"
  style:left="{left}px"
  style:top="{top}px"
>
  {#each entries as entry, index (index)}
    {#if isSeparator(entry)}
      <div role="separator" class="my-1 h-px bg-[var(--border)]"></div>
    {:else}
      {@const Icon = entry.icon}
      <button
        type="button"
        role={entry.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
        aria-checked={entry.checked}
        disabled={entry.disabled}
        tabindex="-1"
        onclick={() => run(entry)}
        onpointerenter={(event) => event.currentTarget.focus()}
        class="w-full flex items-center gap-2.5 px-3 py-1.5 text-left outline-none focus:bg-[var(--bg-hover)] disabled:opacity-45 {entry.danger ? 'text-[var(--danger)]' : 'focus:text-[var(--accent-light)]'}"
      >
        <span class="w-4 shrink-0 flex items-center justify-center text-[var(--text-dim)]" aria-hidden="true">
          {#if entry.checked}<Check size={14} />{:else if Icon}<Icon size={14} />{/if}
        </span>
        <span class="flex-1 truncate">{entry.label}</span>
        {#if entry.shortcut}<span class="ml-4 shrink-0 text-[10px] text-[var(--text-dim)]">{entry.shortcut}</span>{/if}
      </button>
    {/if}
  {/each}
</div>
