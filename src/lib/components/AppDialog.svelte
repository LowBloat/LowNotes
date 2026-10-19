<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/i18n';
  import { dismissibleModal } from '$lib/modal-dismiss';
  import { dismissValue, settleDialog, type DialogRequest } from '$lib/dialogs.svelte';

  let { request } = $props<{ request: DialogRequest }>();
  let value = $state(untrack(() => request.value ?? ''));
  const messageId = `app-dialog-${Math.random().toString(36).slice(2)}`;

  function submit(event: SubmitEvent) {
    event.preventDefault();
    settleDialog(request.kind === 'prompt' ? value : true);
  }
</script>

<div
  class="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 p-4"
  use:dismissibleModal={() => settleDialog(dismissValue(request.kind))}
>
  <form
    role={request.kind === 'alert' ? 'alertdialog' : 'dialog'}
    aria-modal="true"
    aria-labelledby={messageId}
    onsubmit={submit}
    class="w-full max-w-sm rounded-xl border border-[var(--border)] bg-[var(--bg-card)] p-5 shadow-2xl text-sm text-[var(--text-main)] flex flex-col gap-4"
  >
    <p id={messageId} class="leading-relaxed whitespace-pre-line break-words">{request.message}</p>
    {#if request.kind === 'prompt'}
      <input
        bind:value
        aria-labelledby={messageId}
        class="w-full rounded-lg border border-[var(--border)] bg-[var(--bg-main)] px-3 py-2 text-sm focus:outline-none focus:border-[var(--accent)]"
      />
    {/if}
    <div class="flex justify-end gap-2">
      {#if request.kind !== 'alert'}
        <button
          type="button"
          data-autofocus={request.danger ? '' : undefined}
          onclick={() => settleDialog(dismissValue(request.kind))}
          class="px-3 py-1.5 rounded-lg hover:bg-[var(--bg-hover)] text-[var(--text-muted)]"
        >{$t('dialog.cancel')}</button>
      {/if}
      <button
        type="submit"
        data-autofocus={request.kind !== 'prompt' && !request.danger ? '' : undefined}
        class="px-3 py-1.5 rounded-lg font-semibold {request.danger ? 'bg-[var(--danger)] text-white' : 'bg-[var(--accent)] text-black hover:bg-[var(--accent-hover)]'}"
      >{request.confirmLabel ?? $t('dialog.ok')}</button>
    </div>
  </form>
</div>
