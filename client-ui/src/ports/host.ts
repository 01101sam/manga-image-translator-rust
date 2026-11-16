import { host } from "nsh";
import { parseImportSources, type ImportSource, type PickSource } from "../domain/import";
import type { DaemonRef, JobOverrides, LocalFile } from "../domain/types";

/**
 * 宿主协议的类型化封装, 契约见 client-ui/HOST_PROTOCOL.md。
 * Web 由 web/host.js 实现, App 由 Swift 宿主实现; TSX 只有这一份调用代码。
 */

export interface HostSnapshot {
  readonly daemons: readonly DaemonRef[];
  /** 宿主已保存的配对 (Keychain)。Web 为 null, Token 由 TSX 存 NSH Storage。 */
  readonly paired: { readonly daemon: DaemonRef; readonly token: string } | null;
  /** 宿主支持的导入入口, 导入菜单据此决定显示哪些项。 */
  readonly imports: readonly ImportSource[];
}

export type UploadOutcome =
  | { readonly ok: true; readonly jobId: string }
  | { readonly ok: false; readonly kind: "queue-full" | "engine-stopped" | "unauthorized" | "rejected" | "network"; readonly message: string };

export interface UploadRequest {
  readonly file: LocalFile;
  readonly baseUrl: string;
  readonly token: string;
  readonly overrides: JobOverrides;
}

export interface HostPort {
  snapshot(): Promise<HostSnapshot>;
  saveToken(daemon: DaemonRef, token: string): Promise<void>;
  clearToken(daemon: DaemonRef): Promise<void>;
  pickFiles(limit: number, source: PickSource): Promise<LocalFile[]>;
  /** 读系统剪贴板里的图片 (PDF 走 pdf.added)。宿主声明了 paste 才会被调用。 */
  paste(): Promise<LocalFile[]>;
  release(ids: readonly string[]): void;
  upload(request: UploadRequest): Promise<UploadOutcome>;
  saveArtifact(url: string, filename: string): Promise<void>;
  onDaemons(listener: (daemons: readonly DaemonRef[]) => void): () => void;
  onFilesAdded(listener: (files: readonly LocalFile[]) => void): () => void;
  onDragState(listener: (active: boolean) => void): () => void;
  onNotice(listener: (text: string) => void): () => void;
}

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null;

export function parseDaemons(value: unknown): DaemonRef[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((raw) =>
    isRecord(raw) && typeof raw.name === "string" && typeof raw.url === "string"
      ? [{ name: raw.name, url: raw.url.replace(/\/+$/, "") }]
      : [],
  );
}

export function parseFiles(value: unknown): LocalFile[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((raw) => {
    if (!isRecord(raw) || typeof raw.id !== "string" || typeof raw.name !== "string") return [];
    const modifiedAt = typeof raw.modifiedAt === "number" && raw.modifiedAt > 0 && Number.isFinite(raw.modifiedAt) ? raw.modifiedAt : undefined;
    return [
      {
        id: raw.id,
        name: raw.name,
        size: typeof raw.size === "number" ? raw.size : 0,
        type: typeof raw.type === "string" ? raw.type : "",
        ...(modifiedAt === undefined ? {} : { modifiedAt }),
      },
    ];
  });
}

function parseSnapshot(value: unknown): HostSnapshot {
  const daemons = isRecord(value) ? parseDaemons(value.daemons) : [];
  const imports = parseImportSources(isRecord(value) ? value.imports : undefined);
  const rawPaired = isRecord(value) ? value.paired : null;
  if (isRecord(rawPaired) && typeof rawPaired.token === "string") {
    const [daemon] = parseDaemons([rawPaired.daemon]);
    if (daemon) return { daemons, paired: { daemon, token: rawPaired.token }, imports };
  }
  return { daemons, paired: null, imports };
}

const UPLOAD_KINDS = ["queue-full", "engine-stopped", "unauthorized", "rejected", "network"] as const;

function parseOutcome(value: unknown): UploadOutcome {
  if (isRecord(value) && value.ok === true && typeof value.jobId === "string") return { ok: true, jobId: value.jobId };
  const kind = isRecord(value) ? UPLOAD_KINDS.find((k) => k === value.kind) : undefined;
  const message = isRecord(value) && typeof value.message === "string" ? value.message : "上传失败";
  return { ok: false, kind: kind ?? "network", message };
}

/** 上传可能持续很久 (大图、慢网), 取消 host.call 默认的 30 秒超时。 */
const NO_TIMEOUT = { timeout: 0 } as const;

export function createHostPort(): HostPort {
  const listen = <T>(event: string, parse: (payload: unknown) => T, listener: (value: T) => void) =>
    host.on(event, (payload) => listener(parse(payload)));
  return {
    snapshot: async () => parseSnapshot(await host.call("session.snapshot")),
    saveToken: async (daemon, token) => {
      await host.call("session.saveToken", { daemon, token });
    },
    clearToken: async (daemon) => {
      await host.call("session.clearToken", { daemon });
    },
    pickFiles: async (limit, source) => {
      // 用户可能在选择器里停很久, 不设超时。
      const answer = await host.call("files.pick", { limit, source }, NO_TIMEOUT);
      return parseFiles(isRecord(answer) ? answer.files : []);
    },
    paste: async () => {
      const answer = await host.call("files.paste", undefined, NO_TIMEOUT);
      return parseFiles(isRecord(answer) ? answer.files : []);
    },
    release: (ids) => {
      if (ids.length > 0) host.post("files.release", { ids });
    },
    upload: async ({ file, baseUrl, token, overrides }) => {
      try {
        return parseOutcome(
          await host.call("upload.start", { fileId: file.id, url: `${baseUrl}/jobs`, token, overrides }, NO_TIMEOUT),
        );
      } catch (error) {
        return { ok: false, kind: "network", message: error instanceof Error ? error.message : String(error) };
      }
    },
    saveArtifact: async (url, filename) => {
      await host.call("artifact.save", { url, filename }, NO_TIMEOUT);
    },
    onDaemons: (listener) => listen("session.daemons", parseDaemons, listener),
    onFilesAdded: (listener) => listen("files.added", parseFiles, listener),
    onDragState: (listener) => listen("files.dragState", (p) => isRecord(p) && p.active === true, listener),
    onNotice: (listener) => listen("host.notice", (p) => (isRecord(p) && typeof p.text === "string" ? p.text : ""), (text) => text && listener(text)),
  };
}