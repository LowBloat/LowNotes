<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { getVersion } from '@tauri-apps/api/app';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { ArrowLeft, Bot, Check, Download, Globe2, Monitor, NotebookPen, Palette, Pencil, Plus, Settings2, Sparkles, Trash2 } from 'lucide-svelte';
  import { fetchAiModels, retryCredentials, saveAiSettings, saveCloseToTray, saveImageUploadSettings, saveLanguage, saveLineWrapping, saveTheme, saveThemePalettes, saveUpdatePrefs, saveViewMode, saveWebSearchSettings } from '$lib/api';
  import { LOCALE_LABELS, SUPPORTED_LOCALES, t, trError, type LocaleCode } from '$lib/i18n';
  import { BUILTIN_PALETTES, DEFAULT_PALETTE_ID, TOKEN_GROUPS, applyTheme, isHexColor, newCustomPalette, resolvePalette, themeVarsStyle, type ThemeToken } from '$lib/themes';
  import type { AiProviderConfig, AiSettings, AppSettings, AppTheme, ImageUploadProvider, ThemePalette, ThemePalettesSettings, ViewMode, WebSearchSettings } from '$lib/types';
  import { version as packageVersion } from '../../../package.json';

  type Tab = 'general' | 'themes' | 'ai' | 'providers' | 'web' | 'about';
  let { settings, initialTab = 'general', onClose, onChange } = $props<{
    settings: AppSettings;
    initialTab?: Tab;
    onClose: () => void;
    onChange: (settings: AppSettings) => void;
  }>();

  let tab = $state<Tab>(untrack(() => initialTab));
  let aiDraft = $state<AiSettings>(untrack(() => $state.snapshot(settings.ai)));
  let webDraft = $state<WebSearchSettings>(untrack(() => $state.snapshot(settings.web_search)));
  let imgurClientId = $state(untrack(() => settings.image_upload?.imgur_client_id ?? ''));
  const imageProvider = $derived(settings.image_upload?.provider ?? 'local');
  const webSources: Record<string, { name: string; keyless: boolean; keyUrl?: string }> = {
    firecrawl: { name: 'Firecrawl', keyless: true, keyUrl: 'https://www.firecrawl.dev/' },
    keenable: { name: 'Keenable', keyless: true, keyUrl: 'https://keenable.ai/console' },
    exa: { name: 'Exa MCP', keyless: true, keyUrl: 'https://dashboard.exa.ai/api-keys' },
    duckduckgo: { name: 'DuckDuckGo', keyless: true },
    searxng: { name: 'SearXNG', keyless: true },
    brave: { name: 'Brave Search', keyless: false, keyUrl: 'https://api-dashboard.search.brave.com/' },
    parallel: { name: 'Parallel', keyless: false, keyUrl: 'https://platform.parallel.ai/' },
  };
  let selectedProviderId = $state(untrack(() => settings.ai.active_provider_id || settings.ai.providers[0]?.id || ''));
  let provider = $derived(aiDraft.providers.find((item) => item.id === selectedProviderId));
  let version = $state(packageVersion);
  let error = $state('');
  let saved = $state(false);
  let busy = $state(false);
  let models = $state<string[]>([]);
  let loadingModels = $state(false);
  let showKey = $state(false);
  let addingProvider = $state(false);
  let newName = $state('');
  let newUrl = $state('');
  let palettes = $state<ThemePalettesSettings>(untrack(() => $state.snapshot(settings.theme_palettes)));
  let editingPalette = $state<ThemePalette | null>(null);
  let editingIsNew = $state(false);
  let editMode = $state<AppTheme>(untrack(() => settings.theme));
  const allPalettes = $derived([...BUILTIN_PALETTES, ...palettes.custom_palettes]);
  const appMode = $derived<AppTheme>(settings.theme);

  onMount(() => {
    getVersion().then((value) => version = value).catch(() => {});
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || document.querySelector('[data-modal-backdrop]')) return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  });

  async function persist(action: Promise<void>, next: AppSettings) {
    error = '';
    saved = false;
    busy = true;
    try {
      await action;
      onChange(next);
      saved = true;
      setTimeout(() => saved = false, 2500);
    } catch (reason) {
      error = trError(String(reason));
    } finally {
      busy = false;
    }
  }

  async function retryCredentialStore() {
    busy = true; error = '';
    try {
      const next = await retryCredentials();
      aiDraft = $state.snapshot(next.ai);
      webDraft = $state.snapshot(next.web_search);
      onChange(next);
    } catch (reason) { error = trError(String(reason)); }
    finally { busy = false; }
  }

  function changeTheme(value: AppTheme) {
    void persist(saveTheme(value), { ...settings, theme: value });
  }
  function changeLanguage(value: LocaleCode) {
    void persist(saveLanguage(value), { ...settings, language: value });
  }
  function changeViewMode(value: ViewMode) {
    void persist(saveViewMode(value), { ...settings, view_mode: value });
  }
  async function changeLineWrapping(control: HTMLInputElement) {
    const value = control.checked;
    await persist(saveLineWrapping(value), { ...settings, line_wrapping: value });
    control.checked = settings.line_wrapping ?? true;
  }
  function changeTray(value: boolean) {
    void persist(saveCloseToTray(value), { ...settings, close_to_tray: value });
  }
  function changeUpdates(value: boolean) {
    void persist(saveUpdatePrefs(value, settings.skipped_version), { ...settings, update_check: value });
  }
  function saveAi() {
    const nextAi = $state.snapshot(aiDraft);
    void persist(saveAiSettings(nextAi), { ...settings, ai: nextAi });
  }
  function saveWeb() {
    const nextWeb = $state.snapshot(webDraft);
    void persist(saveWebSearchSettings(nextWeb), { ...settings, web_search: nextWeb });
  }
  async function changeImageProvider(control: HTMLSelectElement) {
    const next = { provider: control.value as ImageUploadProvider, imgur_client_id: settings.image_upload?.imgur_client_id ?? '', local_default_applied: true };
    await persist(saveImageUploadSettings(next), { ...settings, image_upload: next });
    control.value = settings.image_upload?.provider ?? 'local';
  }
  function saveImgurClientId() {
    const next = { provider: imageProvider, imgur_client_id: imgurClientId.trim(), local_default_applied: true };
    void persist(saveImageUploadSettings(next), { ...settings, image_upload: next });
  }
  function addProvider() {
    if (!newName.trim() || !newUrl.trim()) return;
    const item: AiProviderConfig = {
      id: `custom_${crypto.randomUUID()}`,
      name: newName.trim(), base_url: newUrl.trim(), api_key: '', selected_model: '', is_custom: true,
    };
    aiDraft.providers.push(item);
    selectedProviderId = item.id;
    aiDraft.active_provider_id = item.id;
    models = [];
    newName = '';
    newUrl = '';
    addingProvider = false;
  }
  function removeProvider(id: string) {
    aiDraft.providers = aiDraft.providers.filter((item) => item.id !== id);
    selectedProviderId = aiDraft.providers[0]?.id || '';
    if (aiDraft.active_provider_id === id) aiDraft.active_provider_id = selectedProviderId;
    models = [];
  }

  function persistPalettes(next: ThemePalettesSettings) {
    applyTheme(next.active_palette_id, settings.theme, next.custom_palettes);
    void persist(saveThemePalettes(next), { ...settings, theme_palettes: next });
  }
  function selectPalette(id: string) {
    if (id === palettes.active_palette_id) return;
    palettes.active_palette_id = id;
    persistPalettes({ active_palette_id: id, custom_palettes: $state.snapshot(palettes.custom_palettes) });
  }
  function startNewPalette() {
    editingPalette = newCustomPalette(resolvePalette(palettes.active_palette_id, palettes.custom_palettes));
    editingIsNew = true;
    editMode = settings.theme;
  }
  function editPalette(palette: ThemePalette) {
    editingPalette = { id: palette.id, name: palette.name, dark: { ...palette.dark }, light: { ...palette.light } };
    editingIsNew = false;
    editMode = settings.theme;
  }
  function savePalette() {
    if (!editingPalette) return;
    const draft = $state.snapshot(editingPalette);
    const name = draft.name.trim();
    if (!name) return;
    const customs = $state.snapshot(palettes.custom_palettes);
    const nextPalette = { ...draft, name };
    const index = customs.findIndex((item) => item.id === draft.id);
    if (index >= 0) customs[index] = nextPalette; else customs.push(nextPalette);
    const activate = editingIsNew || palettes.active_palette_id === draft.id;
    editingPalette = null;
    palettes.custom_palettes = customs;
    if (activate) palettes.active_palette_id = draft.id;
    persistPalettes({ active_palette_id: palettes.active_palette_id, custom_palettes: customs });
  }
  function deletePalette(id: string) {
    if (editingPalette?.id === id) editingPalette = null;
    palettes.custom_palettes = $state.snapshot(palettes.custom_palettes).filter((item) => item.id !== id);
    if (palettes.active_palette_id === id) palettes.active_palette_id = DEFAULT_PALETTE_ID;
    persistPalettes({ active_palette_id: palettes.active_palette_id, custom_palettes: $state.snapshot(palettes.custom_palettes) });
  }
  function commitHex(token: ThemeToken, value: string) {
    if (!editingPalette) return;
    const trimmed = value.trim();
    const normalized = trimmed.startsWith('#') ? trimmed : `#${trimmed}`;
    if (isHexColor(normalized)) editingPalette[editMode][token] = normalized.toLowerCase();
  }
  async function loadModels() {
    if (!provider) return;
    error = '';
    loadingModels = true;
    try {
      models = await fetchAiModels(provider.id, provider.base_url, provider.api_key);
      if (models.length && !provider.selected_model) provider.selected_model = models[0];
    } catch (reason) {
      error = trError(String(reason));
    } finally {
      loadingModels = false;
    }
  }
</script>

<main class="settings-page">
  <header class="settings-topbar">
    <button class="settings-back" onclick={onClose} aria-label={$t('settings.back')}><ArrowLeft size={18} /> {$t('settings.back')}</button>
    <span class="settings-version">LowNotes {version}</span>
  </header>

  <div class="settings-shell">
    <div class="settings-heading">
      <div class="settings-emblem"><Settings2 size={26} strokeWidth={1.7} /></div>
      <div><h1>{$t('settings.title')}</h1><p>{$t('settings.subtitle')}</p></div>
    </div>

    <div class="settings-layout">
      <nav class="settings-nav" aria-label={$t('settings.title')}>
        <button class:active={tab === 'general'} onclick={() => tab = 'general'}><Monitor size={18} /> {$t('settings.general')}</button>
        <button class:active={tab === 'themes'} onclick={() => tab = 'themes'}><Palette size={18} /> {$t('settings.themes')}</button>
        <button class:active={tab === 'ai'} onclick={() => tab = 'ai'}><Sparkles size={18} /> {$t('settings.ai')}</button>
        <button class:active={tab === 'providers'} onclick={() => tab = 'providers'}><Bot size={18} /> {$t('settings.providers')}</button>
        <button class:active={tab === 'web'} onclick={() => tab = 'web'}><Globe2 size={18} /> {$t('settings.webSearch')}</button>
        <button class:active={tab === 'about'} onclick={() => tab = 'about'}><Download size={18} /> {$t('settings.about')}</button>
      </nav>

      <div class="settings-content">
        {#if tab === 'general'}
          <section class="settings-section">
            <h2>{$t('settings.appearance')}</h2><p>{$t('settings.appearanceHint')}</p>
            <div class="setting-row"><div><strong>{$t('settings.theme')}</strong><small>{$t('settings.themeHint')}</small></div>
              <div class="settings-segment"><button class:active={settings.theme === 'dark'} onclick={() => changeTheme('dark')}>{$t('settings.dark')}</button><button class:active={settings.theme === 'light'} onclick={() => changeTheme('light')}>{$t('settings.light')}</button></div></div>
            <div class="setting-row"><div><strong>{$t('settings.language')}</strong><small>{$t('settings.languageHint')}</small></div>
              <select value={settings.language || 'en-US'} onchange={(event) => changeLanguage(event.currentTarget.value as LocaleCode)}>
                {#each SUPPORTED_LOCALES as code}<option value={code}>{LOCALE_LABELS[code]}</option>{/each}
              </select></div>
            <div class="setting-row"><div><strong>{$t('settings.defaultView')}</strong><small>{$t('settings.defaultViewHint')}</small></div>
              <select value={settings.view_mode} onchange={(event) => changeViewMode(event.currentTarget.value as ViewMode)}>
                <option value="edit">{$t('editor.modeEdit')}</option><option value="split">{$t('editor.modeSplit')}</option><option value="preview">{$t('editor.modePreview')}</option>
              </select></div>
            <label class="setting-row setting-toggle"><div><strong>{$t('settings.lineWrapping')}</strong><small>{$t('settings.lineWrappingHint')}</small></div><input type="checkbox" checked={settings.line_wrapping ?? true} disabled={busy} onchange={(event) => void changeLineWrapping(event.currentTarget)} /></label>
          </section>
          <section class="settings-section">
            <h2>{$t('settings.behavior')}</h2><p>{$t('settings.behaviorHint')}</p>
            <label class="setting-row setting-toggle"><div><strong>{$t('settings.closeToTray')}</strong><small>{$t('settings.closeToTrayHint')}</small></div><input type="checkbox" checked={settings.close_to_tray} onchange={(event) => changeTray(event.currentTarget.checked)} /></label>
          </section>
          <section class="settings-section">
            <h2>{$t('settings.pastedImages')}</h2><p>{$t('settings.pastedImagesHint')}</p>
            <div class="setting-row"><div><label for="image-upload-provider"><strong>{$t('settings.imageUploadProvider')}</strong></label><small>{$t('settings.imageUploadProviderHint')}</small></div>
              <select id="image-upload-provider" value={imageProvider} disabled={busy} onchange={(event) => void changeImageProvider(event.currentTarget)}>
                <option value="local">{$t('settings.localImages')}</option><option value="catbox">Catbox</option><option value="imgur">Imgur</option>
              </select></div>
            <p>{$t(imageProvider === 'local' ? 'settings.localImagesHint' : imageProvider === 'imgur' ? 'settings.imgurImagesHint' : 'settings.catboxImagesHint')}</p>
            {#if imageProvider === 'imgur'}
              <label class="setting-field">{$t('settings.imgurClientId')}<input bind:value={imgurClientId} placeholder={$t('settings.imgurClientIdPlaceholder')} maxlength="128" disabled={busy} autocomplete="off" spellcheck="false" /></label>
              <p>{$t('settings.imgurClientIdHint')}</p>
              <div class="settings-actions"><button class="settings-primary" onclick={saveImgurClientId} disabled={busy}>{$t('settings.saveImageUpload')}</button></div>
            {/if}
          </section>
        {:else if tab === 'themes'}
          <section class="settings-section">
            <div class="settings-section-title">
              <div><h2>{$t('settings.colorTheme')}</h2><p>{$t('settings.colorThemeHint')}</p></div>
              <button class="settings-primary" onclick={startNewPalette}><Plus size={16} /> {$t('settings.addTheme')}</button>
            </div>
            <div class="palette-grid">
              {#each allPalettes as palette (palette.id)}
                {@const custom = !BUILTIN_PALETTES.some((builtin) => builtin.id === palette.id)}
                {@const colors = palette[appMode]}
                <div class="palette-card" class:active={palettes.active_palette_id === palette.id}>
                  <button class="palette-select" onclick={() => selectPalette(palette.id)} aria-pressed={palettes.active_palette_id === palette.id}>
                    <span class="palette-swatches">
                      {#each [colors.bg_sidebar, colors.bg_card, colors.accent, colors.text_main, colors.danger] as swatch}<span style="background: {swatch}"></span>{/each}
                    </span>
                    <span class="palette-meta">
                      <span><span class="palette-name">{palette.name}</span><span class="palette-sub">{custom ? $t('settings.paletteCustom') : $t('settings.paletteBuiltIn')}</span></span>
                      {#if palette.id === DEFAULT_PALETTE_ID}<span class="palette-badge accent">{$t('settings.paletteDefault')}</span>{:else if palettes.active_palette_id === palette.id}<span class="palette-badge">{$t('settings.inUse')}</span>{/if}
                    </span>
                  </button>
                  {#if custom}
                    <span class="palette-tools">
                      <button onclick={() => editPalette(palette)} title={$t('settings.editTheme')} aria-label={$t('settings.editTheme')}><Pencil size={14} /></button>
                      <button class="danger" onclick={() => deletePalette(palette.id)} title={$t('settings.deleteTheme')} aria-label={$t('settings.deleteTheme')}><Trash2 size={14} /></button>
                    </span>
                  {/if}
                </div>
              {/each}
            </div>
            {#if editingPalette}
              <div class="palette-editor">
                <div class="palette-editor-head">
                  <div><h3>{editingIsNew ? $t('settings.addTheme') : $t('settings.editTheme')}</h3><small>{$t('settings.editColorsHint')}</small></div>
                  <div class="settings-segment">
                    <button class:active={editMode === 'dark'} onclick={() => editMode = 'dark'}>{$t('settings.dark')}</button>
                    <button class:active={editMode === 'light'} onclick={() => editMode = 'light'}>{$t('settings.light')}</button>
                  </div>
                </div>
                <label class="setting-field palette-field-name">{$t('settings.themeName')}<input bind:value={editingPalette.name} placeholder={$t('settings.themeNamePlaceholder')} maxlength="40" /></label>
                <div class="palette-editor-body">
                  <div>
                    {#each TOKEN_GROUPS as group (group.group)}
                      <div class="palette-group">
                        <h4>{$t(group.group)}</h4>
                        <div class="palette-picker-grid">
                          {#each group.tokens as item (item.token)}
                            <div class="palette-picker">
                              <input type="color" aria-label={$t(item.labelKey)} bind:value={editingPalette[editMode][item.token]} />
                              <span class="palette-picker-label">
                                <strong>{$t(item.labelKey)}</strong>
                                <input value={editingPalette[editMode][item.token]} onchange={(event) => commitHex(item.token, event.currentTarget.value)} spellcheck="false" />
                              </span>
                            </div>
                          {/each}
                        </div>
                      </div>
                    {/each}
                  </div>
                  <div class="palette-demo" style={themeVarsStyle(editingPalette[editMode], editMode)}>
                    <div class="palette-demo-bar"><span>LowNotes</span><NotebookPen size={13} /></div>
                    <div class="palette-demo-body">
                      <div class="palette-demo-side">
                        <span class="palette-demo-item active">{$t('settings.demoNote')}</span>
                        <span class="palette-demo-item">Explosion!</span>
                        <span class="palette-demo-item">Slime</span>
                      </div>
                      <div class="palette-demo-main">
                        <div class="palette-demo-note">
                          <h5>{$t('settings.demoNote')}</h5>
                          <p>{$t('settings.demoBody')} <span class="palette-demo-link">{$t('settings.demoLink')}</span></p>
                          <div class="palette-demo-row">
                            <span class="palette-demo-btn">{$t('settings.demoButton')}</span>
                            <span class="palette-demo-chip ok">{$t('settings.demoSuccess')}</span>
                            <span class="palette-demo-chip bad">{$t('settings.demoDanger')}</span>
                            <span class="palette-demo-swatch"></span>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
                <div class="settings-actions">
                  <button onclick={() => editingPalette = null}>{$t('ai.cancel')}</button>
                  <button class="settings-primary" onclick={savePalette} disabled={busy || !editingPalette.name.trim()}><Check size={15} /> {$t('settings.saveTheme')}</button>
                </div>
              </div>
            {/if}
          </section>
        {:else if tab === 'ai'}
          <section class="settings-section">
            <h2>{$t('settings.ai')}</h2><p>{$t('settings.aiHint')}</p>
            <div class="setting-row"><div><strong>{$t('settings.activeProvider')}</strong><small>{$t('settings.activeProviderHint')}</small></div>
              <select bind:value={aiDraft.active_provider_id}>{#each aiDraft.providers as item}<option value={item.id}>{item.name}</option>{/each}</select></div>
            <label class="setting-row setting-toggle"><div><strong>{$t('ai.autoLink')}</strong><small>{$t('ai.autoLinkHint')}</small></div><input type="checkbox" bind:checked={aiDraft.auto_link_notes} /></label>
            <div class="settings-actions"><button class="settings-primary" onclick={saveAi} disabled={busy}>{$t('settings.saveAi')}</button></div>
          </section>
        {:else if tab === 'providers'}
          <section class="settings-section">
            <div class="settings-section-title"><div><h2>{$t('settings.providers')}</h2><p>{$t('settings.providersHint')}</p></div><button class="settings-primary" onclick={() => addingProvider = !addingProvider}><Plus size={16} /> {$t('settings.addProvider')}</button></div>
            {#if addingProvider}<div class="settings-add-form"><label>{$t('ai.providerNamePlaceholder')}<input bind:value={newName} /></label><label>{$t('ai.providerUrlPlaceholder')}<input bind:value={newUrl} placeholder="http://localhost:11434/v1" /></label><div class="settings-actions"><button onclick={() => addingProvider = false}>{$t('ai.cancel')}</button><button class="settings-primary" onclick={addProvider} disabled={!newName.trim() || !newUrl.trim()}>{$t('ai.add')}</button></div></div>{/if}
            <div class="provider-workspace">
              <div class="provider-list" aria-label={$t('settings.providers')}>
                {#each aiDraft.providers as item}<button class:active={selectedProviderId === item.id} onclick={() => { selectedProviderId = item.id; models = []; }}><span>{item.name}</span>{#if aiDraft.active_provider_id === item.id}<Check size={15} />{/if}</button>{/each}
              </div>
              {#if provider}<div class="provider-detail">
                <div class="provider-detail-head"><div><h3>{provider.name}</h3><small>{provider.is_custom ? $t('settings.customProvider') : $t('settings.builtInProvider')}</small></div>{#if provider.is_custom}<button class="settings-icon danger" onclick={() => removeProvider(provider.id)} title={$t('settings.removeProvider')} aria-label={$t('settings.removeProvider')}><Trash2 size={17} /></button>{/if}</div>
                <label class="setting-field">{$t('ai.baseUrl')}<input bind:value={provider.base_url} placeholder="http://localhost:11434/v1" /></label>
                <label class="setting-field">{$t('ai.apiKey')}<div class="settings-inline"><input type={showKey ? 'text' : 'password'} bind:value={provider.api_key} autocomplete="off" /><button onclick={() => showKey = !showKey}>{showKey ? $t('ai.hide') : $t('ai.show')}</button></div></label>
                <label class="setting-field">{$t('ai.selectedModel')}<input bind:value={provider.selected_model} placeholder={$t('ai.modelPlaceholder')} /></label>
                <div class="settings-models"><button onclick={loadModels} disabled={loadingModels}>{loadingModels ? $t('ai.fetching') : $t('ai.listModels')}</button>{#if models.length}<select value={provider.selected_model} onchange={(event) => provider.selected_model = event.currentTarget.value}><option value="">{$t('settings.chooseModel')}</option>{#each models as model}<option value={model}>{model}</option>{/each}</select>{/if}</div>
                <label class="settings-active-choice"><input type="radio" name="active-provider" checked={aiDraft.active_provider_id === provider.id} onchange={() => aiDraft.active_provider_id = provider.id} /> {$t('settings.useProvider')}</label>
              </div>{/if}
            </div>
            <div class="settings-actions"><button class="settings-primary" onclick={saveAi} disabled={busy}>{$t('settings.saveProviders')}</button></div>
          </section>
        {:else if tab === 'web'}
          <section class="settings-section">
            <h2>{$t('settings.webSearch')}</h2><p>{$t('settings.webSearchHint')}</p>
            <div class="web-route-note"><Globe2 size={19} /><span>{$t('settings.webRouteHint')}</span></div>
            <div class="web-source-list">
              {#each webDraft.sources as source (source.id)}
                {@const info = webSources[source.id]}
                {#if info}
                  <div class="web-source-row">
                    <label class="web-source-head"><span class="web-source-title"><strong>{info.name}</strong><small>{info.keyless ? $t('settings.keyless') : $t('settings.requiresKey')}</small></span><input type="checkbox" bind:checked={source.enabled} /></label>
                    {#if source.id === 'searxng'}
                      <label class="setting-field">{$t('settings.publicInstance')}<input type="url" bind:value={webDraft.searxng_url} placeholder="https://search.lumy.live/" /><small>{$t('settings.publicInstanceHint')}</small></label>
                    {/if}
                    {#if info.keyUrl}
                      <label class="setting-field">{info.keyless ? $t('settings.optionalKey') : $t('settings.apiKey')}<input type="password" bind:value={source.api_key} autocomplete="off" placeholder={info.keyless ? $t('settings.optionalKey') : $t('settings.requiresKey')} /></label>
                      <button class="settings-link" onclick={() => openUrl(info.keyUrl!)}>{$t('settings.getKey')}</button>
                    {/if}
                  </div>
                {/if}
              {/each}
            </div>
            <div class="settings-actions"><button class="settings-primary" onclick={saveWeb} disabled={busy}>{$t('settings.saveWeb')}</button></div>
          </section>
        {:else}
          <section class="settings-section">
            <h2>{$t('settings.about')}</h2><p>{$t('settings.aboutHint')}</p>
            <div class="settings-about"><div class="settings-about-mark"><NotebookPen size={26} /></div><div><strong>LowNotes</strong><span>{$t('settings.version')} {version}</span></div></div>
            <label class="setting-row setting-toggle"><div><strong>{$t('update.autoCheck')}</strong><small>{$t('settings.updateHint')}</small></div><input type="checkbox" checked={settings.update_check} onchange={(event) => changeUpdates(event.currentTarget.checked)} /></label>
            <div class="setting-row"><div><strong>{$t('settings.device')}</strong><small>{settings.device_name}</small></div></div>
          </section>
        {/if}
        {#if settings.credential_error}
          <div class="settings-error" role="alert"><p>{trError(settings.credential_error)}</p><button class="settings-primary" disabled={busy} onclick={() => void retryCredentialStore()}>{$t('credentials.retry')}</button></div>
        {:else}
          <p class="text-xs text-[var(--text-muted)] mt-4">{$t('credentials.protected')}</p>
        {/if}
        {#if error}<p class="settings-error" role="alert">{error}</p>{/if}
        {#if saved}<p class="settings-saved" role="status"><Check size={15} /> {$t('settings.saved')}</p>{/if}
      </div>
    </div>
  </div>
</main>
