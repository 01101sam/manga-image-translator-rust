import { Text, View } from "nsh";
import { dismissToast, toast } from "../store/toast";

/** 浮在底部的短提示, 点一下收起。 */
export function ToastHost() {
  return (
    <If when={toast()}>
      {(current) => (
        <View
          testID="toast"
          accessibilityLabel={current().text}
          onPress={dismissToast}
          class="absolute bottom-24 left-4 right-4 items-center"
        >
          <View class={`max-w-96 rounded-(--radius) px-4 py-3 shadow-lg ${current().tone === "error" ? "bg-destructive" : "bg-foreground"}`}>
            <Text class={`text-sm ${current().tone === "error" ? "text-destructive-foreground" : "text-background"}`}>{current().text}</Text>
          </View>
        </View>
      )}
    </If>
  );
}
