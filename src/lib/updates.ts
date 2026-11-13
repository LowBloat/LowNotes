import { check, type Update } from '@tauri-apps/plugin-updater';
import { checkExternalUpdate } from './api';
import type { ReleaseNotice, UpdatePolicy } from './types';
import type { TKey } from './i18n';

export interface AvailableUpdate {
  version: string;
  body?: string;
  download_url?: string | null;
  installer?: Update;
  /** Downloaded and verified in the background: installing only restarts. */
  downloaded?: boolean;
}

/** A check that hangs (a network that changed under it, a sleeping laptop)
 * would otherwise keep every later check waiting. */
export const CHECK_TIMEOUT_MS = 30 * 1000;

export async function checkAvailableUpdate(
  policy: UpdatePolicy,
  nativeCheck: typeof check = check,
  externalCheck: () => Promise<ReleaseNotice | null> = checkExternalUpdate,
): Promise<AvailableUpdate | null> {
  if (!policy.can_install) return await externalCheck();
  const update = await nativeCheck({ target: policy.updater_target ?? undefined, timeout: CHECK_TIMEOUT_MS });
  return update ? { version: update.version, body: update.body, installer: update } : null;
}

/** Each native check holds the update (and its download) on the Rust side
 * until closed; checks run every minute, so the ones not kept are released. */
export function discardUpdate(update: AvailableUpdate | null | undefined) {
  update?.installer?.close().catch(() => {});
}

export function canInstallUpdate(policy: UpdatePolicy | null, update: AvailableUpdate | null): boolean {
  return policy?.can_install === true && update?.installer !== undefined;
}

export function updateInstructionKey(policy: UpdatePolicy | null): TKey {
  switch (policy?.channel) {
    case 'arch': return 'update.archHint';
    case 'deb': return 'update.debHint';
    case 'rpm': return 'update.rpmHint';
    case 'flatpak': return 'update.flatpakHint';
    case 'snap': return 'update.snapHint';
    case 'portable': return 'update.portableHint';
    case 'appimage': return 'update.appimageHint';
    default: return 'update.packageHint';
  }
}
