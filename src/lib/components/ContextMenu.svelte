<script lang="ts">
  import { closeContextMenu, contextMenu } from '$lib/context-menu.svelte';
  import { keepsNativeMenu } from '$lib/menu';
  import MenuList from './MenuList.svelte';

  function insideMenu(target: EventTarget | null): boolean {
    return target instanceof Element && !!target.closest('[data-menu]');
  }

  // Areas without a menu of their own get none, rather than the WebView's.
  function suppressNativeMenu(event: MouseEvent) {
    if (!event.defaultPrevented && !keepsNativeMenu(event.target)) event.preventDefault();
  }
</script>

<svelte:window
  oncontextmenu={suppressNativeMenu}
  onpointerdown={(event) => { if (contextMenu.current && !insideMenu(event.target)) closeContextMenu(); }}
  onblur={() => closeContextMenu()}
  onresize={() => closeContextMenu()}
/>
<svelte:document onscrollcapture={(event) => { if (contextMenu.current && !insideMenu(event.target)) closeContextMenu(); }} />

{#if contextMenu.current}
  {#key contextMenu.current}
    <MenuList entries={contextMenu.current.entries} x={contextMenu.current.x} y={contextMenu.current.y} onClose={closeContextMenu} />
  {/key}
{/if}
