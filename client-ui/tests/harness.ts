import { open } from "nsh/test";
import { BASE, type FakeDaemon } from "./fake-daemon";

export interface HostFileRef {
  readonly id: string;
  readonly name: string;
  readonly size: number;
  readonly type: string;
  readonly modifiedAt?: number;
  readonly thumb?: string;
  readonly hash?: string;
  readonly width?: number;
  readonly height?: number;
}

export interface Harness {
  readonly daemon: FakeDaemon;
  /** files.pick 依次交回的结果 (每次点导入菜单取一份)。 */
  readonly picks?: HostFileRef[][];
  /** upload.start 的应答; 缺省交给 Daemon 替身。 */
  readonly upload?: (data: { fileId: string }) => unknown;
  /** 宿主在 files.resolve 里认得的文件 (重启后还留着的待上传)。 */
  readonly resolve?: (ids: readonly string[]) => HostFileRef[];
  readonly storage?: Record<string, string>;
  readonly platform?: "ios" | "web";
  readonly width?: number;
  readonly height?: number;
}

/** 已配对的应用: 宿主 (Swift / host.js) 由 handlers 扮演, Daemon 由 fake 扮演。 */
export async function launchPaired(scenario: Harness) {
  const queue = [...(scenario.picks ?? [])];
  const handlers = {
    "session.snapshot": () => ({
      daemons: [{ name: "studio", url: BASE }],
      paired: { daemon: { name: "studio", url: BASE }, token: scenario.daemon.token() },
    }),
    "session.saveToken": null,
    "session.clearToken": null,
    "files.pick": () => ({ files: queue.shift() ?? [] }),
    "files.paste": () => ({ files: [] }),
    "files.resolve": (data: { ids: string[] }) => ({ files: scenario.resolve ? scenario.resolve(data.ids) : [] }),
    "files.release": null,
    "upload.start": (data: { fileId: string }) => (scenario.upload ? scenario.upload(data) : scenario.daemon.submit()),
    "artifact.save": null,
    "pdf.pick": () => ({ file: null }),
    "pdf.release": null,
    "pdf.show": () => ({ pageCount: 3 }),
    "pdf.dispose": null,
    "pdf.open": () => ({ file: null }),
  };
  return open("../App.tsx", {
    clock: {},
    network: scenario.daemon.network,
    host: { handlers },
    ...(scenario.storage ? { storage: scenario.storage } : {}),
    ...(scenario.platform ? { platform: scenario.platform } : {}),
    ...(scenario.width ? { width: scenario.width, height: scenario.height } : {}),
  });
}

export type Launched = Awaited<ReturnType<typeof launchPaired>>;

export const hostFiles = (n: number, prefix = "p", extra: Partial<HostFileRef> = {}): HostFileRef[] =>
  Array.from({ length: n }, (_, i) => ({ id: prefix + i, name: prefix + i + ".png", size: 1000 + i, type: "image/png", ...extra }));
