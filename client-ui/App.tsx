import { cleanup, effect, mount, untracked, View } from "nsh";
import { PairingScreen } from "./src/pages/PairingScreen";
import { Shell } from "./src/pages/Shell";
import { startApp } from "./src/store/app";
import { startPolling } from "./src/store/jobs";
import { sessionKind } from "./src/store/session";

function App() {
  startApp();
  // 配对成功才轮询; 回到未配对 (401 或取消配对) 时停止。startPolling 内部读的 signal 不应让这个 effect 重跑。
  effect(() => {
    if (sessionKind() !== "paired") return;
    const stop = untracked(startPolling);
    cleanup(stop);
  });
  return (
    <View class="flex-1 bg-background">
      <If when={sessionKind() === "paired"} else={<PairingScreen />}>
        <Shell />
      </If>
    </View>
  );
}

mount(App);
