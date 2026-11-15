import { computed, platform, signal, Storage } from "nsh";
import { createApi, DaemonError, type Api } from "../api/daemon";
import { initialSession, reduceSession, type Session, type SessionCommand, type SessionEvent } from "../domain/session";
import type { DaemonRef } from "../domain/types";
import { createHostPort } from "../ports/host";

/** 宿主端口只有一个实例; 测试通过 open() 的 host.handlers 替换宿主本身。 */
export const hostPort = createHostPort();

export const session = signal<Session>(initialSession);
/** 此刻发现的 Daemon, 「更换 Daemon」回到列表时用。 */
export const knownDaemons = signal<readonly DaemonRef[]>([]);
/** 宿主没有应答或快照不可读时的说明。 */
export const startupError = signal<string | null>(null);

/** 配对页输入框里的 Key。放在 store 里, 页面因状态切换重建时不丢。 */
export const keyDraft = signal("sam01101");
export const sessionKind = computed(() => session().kind);

const TOKEN_KEY = "session.token:";
export const PAIRING_KEY_STORE = "pairing.key";

/** Web 没有 Keychain, Token 存在 NSH Storage (即浏览器 localStorage); App 的 Token 归宿主的 Keychain。 */
const tokensInStorage = platform.os === "web";

/** 已配对时的 API 客户端; 其他状态为 null。会话对象换了才重建。 */
export const api = computed<Api | null>(() => {
  const s = session();
  return s.kind === "paired" ? createApi(s.daemon.url, s.token) : null;
});

export function dispatch(event: SessionEvent) {
  const [next, command] = reduceSession(session(), event);
  if (next !== session()) session.set(next);
  void run(command);
}

async function run(command: SessionCommand): Promise<void> {
  switch (command.type) {
    case "none":
      return;
    case "pair":
      try {
        const token = await createApi(command.daemon.url, null).pair(command.key);
        await Storage.setItem(PAIRING_KEY_STORE, command.key);
        dispatch({ type: "pairSucceeded", token });
      } catch (error) {
        const rejected = error instanceof DaemonError && error.kind === "rejected";
        dispatch({
          type: "pairFailed",
          kind: rejected ? "rejected" : "failed",
          message: rejected ? "Pairing Key 不正确" : error instanceof Error ? error.message : String(error),
        });
      }
      return;
    case "saveToken":
      try {
        if (tokensInStorage) await Storage.setItem(TOKEN_KEY + command.daemon.url, command.token);
        else await hostPort.saveToken(command.daemon, command.token);
      } catch (error) {
        // 保存失败不影响本次使用, 下次启动需要重新配对。
        startupError.set(`无法保存配对信息: ${error instanceof Error ? error.message : String(error)}`);
      }
      return;
    case "clearToken":
      try {
        if (tokensInStorage) await Storage.removeItem(TOKEN_KEY + command.daemon.url);
        else await hostPort.clearToken(command.daemon);
      } catch {
        // 清除失败时, 下次启动会拿到旧 Token 并再收一次 401, 回到这里。
      }
      return;
  }
}

async function savedFromStorage(daemons: readonly DaemonRef[]) {
  for (const daemon of daemons) {
    const token = await Storage.getItem(TOKEN_KEY + daemon.url);
    if (token) return { daemon, token };
  }
  return null;
}

/** 启动: 先挂监听, 再读宿主的初始状态 (宿主在监听挂上之前发的事件不会重放)。 */
export async function startSession(): Promise<void> {
  void Storage.getItem(PAIRING_KEY_STORE).then((saved) => {
    if (saved) keyDraft.set(saved);
  });
  hostPort.onDaemons((daemons) => {
    knownDaemons.set(daemons);
    dispatch({ type: "daemons", daemons });
  });
  try {
    const snapshot = await hostPort.snapshot();
    knownDaemons.set(snapshot.daemons);
    const saved = snapshot.paired ?? (tokensInStorage ? await savedFromStorage(snapshot.daemons) : null);
    if (saved) dispatch({ type: "restored", daemon: saved.daemon, token: saved.token });
    else dispatch({ type: "daemons", daemons: snapshot.daemons });
  } catch (error) {
    startupError.set(`无法读取宿主状态: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export const submitKey = (key: string) => dispatch({ type: "submitKey", key });
export const selectDaemon = (daemon: DaemonRef) => dispatch({ type: "select", daemon });
export const forgetDaemon = () => dispatch({ type: "forget", known: knownDaemons() });

/** 任何受保护请求收到 401 都走这里。 */
export function handleError(error: unknown): boolean {
  if (error instanceof DaemonError && error.kind === "unauthorized") {
    dispatch({ type: "unauthorized" });
    return true;
  }
  return false;
}
