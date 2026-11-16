import { Text, View } from "nsh";
import { dismissToast, pressToastAction, toast } from "../store/toast";
import { Button } from "./button";

/**
 * 浮在底部的短提示, 点文字收起。带按钮的提示里按钮和文字是并排的两个元素: 文字那一块带无障碍标签, 按钮不能放在它里面,
 * 否则系统把整块当成一个元素, 按钮在无障碍树里消失 (实测: XCUITest 找不到按钮)。
 */
export function ToastHost() {
  return (
    <If when={toast()}>
      {(current) => (
        <View class="absolute bottom-24 left-4 right-4 items-center">
          <View class={`max-w-96 flex-row items-center gap-3 rounded-(--radius) px-4 py-3 shadow-lg ${current().tone === "error" ? "bg-destructive" : "bg-foreground"}`}>
            <View testID="toast" accessibilityLabel={current().text} onPress={dismissToast} class="flex-1">
              <Text class={`text-sm ${current().tone === "error" ? "text-destructive-foreground" : "text-background"}`}>{current().text}</Text>
            </View>
            <If when={current().action}>
              {(action) => <Button label={action().label} size="sm" variant="secondary" testID="toast-action" onPress={pressToastAction} />}
            </If>
          </View>
        </View>
      )}
    </If>
  );
}
