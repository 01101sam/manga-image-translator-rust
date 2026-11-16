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
  /** 替换 Daemon 替身的网络桩 (比如让 GET /jobs 暂时连不上)。 */
  readonly network?: FakeDaemon["network"];
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
    network: scenario.network ?? scenario.daemon.network,
    host: { handlers },
    ...(scenario.storage ? { storage: scenario.storage } : {}),
    ...(scenario.platform ? { platform: scenario.platform } : {}),
    ...(scenario.width ? { width: scenario.width, height: scenario.height } : {}),
  });
}

export type Launched = Awaited<ReturnType<typeof launchPaired>>;

export const hostFiles = (n: number, prefix = "p", extra: Partial<HostFileRef> = {}): HostFileRef[] =>
  Array.from({ length: n }, (_, i) => ({ id: prefix + i, name: prefix + i + ".png", size: 1000 + i, type: "image/png", ...extra }));
/** 确定性的 64 位十六进制「指纹」: 同一个 seed 同一个值, 不同 seed 之间差大约一半的位。 */
export function hashFor(seed: number): string {
  let state = (seed * 2654435761) >>> 0;
  let out = "";
  for (let i = 0; i < 8; i++) {
    state = (Math.imul(state, 1103515245) + 12345) >>> 0;
    out += state.toString(16).padStart(8, "0");
  }
  return out;
}

/** 翻转 hash 里的 count 个位 (均匀地选), 得到汉明距离恰好是 count 的另一个哈希。 */
export function flipBits(hash: string, count: number): string {
  const nibbles = hash.split("").map((c) => parseInt(c, 16));
  for (let i = 0; i < count; i++) {
    const bit = (i * 37) % 256;
    nibbles[bit >> 2] = nibbles[bit >> 2]! ^ (8 >> (bit & 3));
  }
  return nibbles.map((n) => n.toString(16)).join("");
}
