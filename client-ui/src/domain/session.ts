import type { DaemonRef } from "./types";

/**
 * 配对会话。Token 一旦保存就永久有效, 只有 Daemon 回 401 才会回到 unpaired。
 * `pairing` 只在用户提交 Pairing Key 到收到应答之间存在, 所以重复点击不会发出第二个请求。
 */
export type Session =
  | { readonly kind: "discovering"; readonly daemons: readonly DaemonRef[] }
  | { readonly kind: "unpaired"; readonly daemon: DaemonRef; readonly notice: PairNotice | null }
  | { readonly kind: "pairing"; readonly daemon: DaemonRef; readonly key: string }
  | { readonly kind: "paired"; readonly daemon: DaemonRef; readonly token: string };

/** unpaired 状态下展示给用户的原因。 */
export interface PairNotice {
  /** rejected: Pairing Key 不对; failed: 连不上或其他错误; revoked: 已保存的 Token 被 Daemon 拒绝。 */
  readonly kind: "rejected" | "failed" | "revoked";
  readonly message: string;
}

export type SessionEvent =
  | { readonly type: "daemons"; readonly daemons: readonly DaemonRef[] }
  | { readonly type: "restored"; readonly daemon: DaemonRef; readonly token: string }
  | { readonly type: "select"; readonly daemon: DaemonRef }
  | { readonly type: "submitKey"; readonly key: string }
  | { readonly type: "pairSucceeded"; readonly token: string }
  | { readonly type: "pairFailed"; readonly kind: "rejected" | "failed"; readonly message: string }
  | { readonly type: "unauthorized" }
  /** 更换 Daemon: `known` 是此刻发现的 Daemon 列表, 由调用方带入以保持 reducer 纯净。 */
  | { readonly type: "forget"; readonly known: readonly DaemonRef[] };

export type SessionCommand =
  | { readonly type: "none" }
  | { readonly type: "pair"; readonly daemon: DaemonRef; readonly key: string }
  | { readonly type: "saveToken"; readonly daemon: DaemonRef; readonly token: string }
  | { readonly type: "clearToken"; readonly daemon: DaemonRef };

const none: SessionCommand = { type: "none" };

export const initialSession: Session = { kind: "discovering", daemons: [] };

export function reduceSession(session: Session, event: SessionEvent): [Session, SessionCommand] {
  switch (event.type) {
    case "daemons": {
      if (session.kind !== "discovering") return [session, none];
      // 只发现一个 Daemon 时直接选它, 省一次点击。
      if (event.daemons.length === 1) {
        return [{ kind: "unpaired", daemon: event.daemons[0]!, notice: null }, none];
      }
      return [{ kind: "discovering", daemons: event.daemons }, none];
    }
    case "restored":
      return [{ kind: "paired", daemon: event.daemon, token: event.token }, none];
    case "select":
      if (session.kind !== "discovering") return [session, none];
      return [{ kind: "unpaired", daemon: event.daemon, notice: null }, none];
    case "submitKey": {
      if (session.kind !== "unpaired") return [session, none];
      const key = event.key.trim();
      if (key === "") {
        return [
          { kind: "unpaired", daemon: session.daemon, notice: { kind: "rejected", message: "请输入 Pairing Key" } },
          none,
        ];
      }
      return [{ kind: "pairing", daemon: session.daemon, key }, { type: "pair", daemon: session.daemon, key }];
    }
    case "pairSucceeded":
      if (session.kind !== "pairing") return [session, none];
      return [
        { kind: "paired", daemon: session.daemon, token: event.token },
        { type: "saveToken", daemon: session.daemon, token: event.token },
      ];
    case "pairFailed":
      if (session.kind !== "pairing") return [session, none];
      return [
        { kind: "unpaired", daemon: session.daemon, notice: { kind: event.kind, message: event.message } },
        none,
      ];
    case "unauthorized":
      if (session.kind !== "paired") return [session, none];
      return [
        {
          kind: "unpaired",
          daemon: session.daemon,
          notice: { kind: "revoked", message: `与 ${session.daemon.name} 的配对已失效, 请重新输入 Pairing Key` },
        },
        { type: "clearToken", daemon: session.daemon },
      ];
    case "forget": {
      const command: SessionCommand =
        session.kind === "paired" ? { type: "clearToken", daemon: session.daemon } : none;
      if (event.known.length === 1) return [{ kind: "unpaired", daemon: event.known[0]!, notice: null }, command];
      return [{ kind: "discovering", daemons: event.known }, command];
    }
  }
}
