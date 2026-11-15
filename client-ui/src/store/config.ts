import { signal } from "nsh";
import type { ConfigDoc } from "../domain/config";
import { isDirty, setIn, validateConfig } from "../domain/config";
import { api, handleError } from "./session";
import { showToast } from "./toast";

export type ConfigLoad = "idle" | "loading" | "ready" | "failed";

export const configLoad = signal<ConfigLoad>("idle");
export const configError = signal<string | null>(null);
/** Daemon 上当前保存的版本。 */
export const configOriginal = signal<ConfigDoc | null>(null);
/** 表单正在编辑的版本。 */
export const configDraft = signal<ConfigDoc | null>(null);
export const configSaving = signal(false);
export const configErrors = signal<ReadonlyMap<string, string>>(new Map());

export const configDirty = (): boolean => isDirty(configOriginal(), configDraft());

export async function loadConfig(): Promise<void> {
  const client = api();
  if (!client || configLoad() === "loading") return;
  configLoad.set("loading");
  configError.set(null);
  try {
    const doc = await client.getConfig();
    configOriginal.set(doc);
    configDraft.set(doc);
    configErrors.set(new Map());
    configLoad.set("ready");
  } catch (error) {
    if (!handleError(error)) configError.set(error instanceof Error ? error.message : String(error));
    configLoad.set("failed");
  }
}

export function editConfig(path: readonly string[], value: unknown) {
  const draft = configDraft();
  if (draft === null) return;
  const next = setIn(draft, path, value);
  configDraft.set(next);
  // 已经报过错的字段, 改动时就地重新校验, 红字随之消失。
  if (configErrors().size > 0) configErrors.set(validateConfig(next));
}

export function discardConfig() {
  configDraft.set(configOriginal());
  configErrors.set(new Map());
}

export async function saveConfig(): Promise<void> {
  const client = api();
  const draft = configDraft();
  if (!client || draft === null || configSaving()) return;
  const errors = validateConfig(draft);
  configErrors.set(errors);
  if (errors.size > 0) {
    showToast("有字段不合法, 请检查标红的项", "error");
    return;
  }
  configSaving.set(true);
  try {
    const saved = await client.putConfig(draft);
    configOriginal.set(saved);
    configDraft.set(saved);
    showToast("配置已保存");
  } catch (error) {
    if (!handleError(error)) showToast(error instanceof Error ? error.message : String(error), "error");
  } finally {
    configSaving.set(false);
  }
}
