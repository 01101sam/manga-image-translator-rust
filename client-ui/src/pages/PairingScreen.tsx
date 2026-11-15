import { Text, View } from "nsh";
import { Button, Card, Icon, Spinner, TextField } from "../ui";
import { keyDraft, knownDaemons, selectDaemon, session, sessionKind, startupError, submitKey } from "../store/session";

/** 未配对时的整页: 发现 Daemon → 输入 Pairing Key。成功一次后 Token 永久保存, 不会再看到这页。 */
export function PairingScreen() {
  return (
    <View class="flex-1 items-center justify-center bg-background p-6 pt-[env(safe-area-inset-top)]">
      <View class="w-full max-w-96 gap-6">
        <View class="items-center gap-3">
          <View class="size-14 items-center justify-center rounded-2xl bg-primary">
            <Icon name="image" size={28} tone="primary-foreground" />
          </View>
          <Text class="text-2xl font-semibold text-foreground">图片翻译</Text>
        </View>
        <If when={sessionKind() === "discovering"}>
          <Discovering />
        </If>
        <If when={sessionKind() === "unpaired"}>
          <KeyForm />
        </If>
        <If when={sessionKind() === "pairing"}>
          <Card class="items-center py-8">
            <Spinner label="正在配对…" />
          </Card>
        </If>
      </View>
    </View>
  );
}

function Discovering() {
  return (
    <Card>
      <If
        when={startupError()}
        else={
          <If
            when={knownDaemons().length > 0}
            else={
              <View class="items-center gap-3 py-4">
                <Spinner label="正在搜索 Daemon…" />
                <Text class="text-center text-sm text-muted-foreground">请确认 Daemon 已在局域网内启动。</Text>
              </View>
            }
          >
            <Text class="text-sm font-medium text-foreground">选择要连接的 Daemon</Text>
            <Each of={knownDaemons()} key={(d) => d.url}>
              {(daemon) => (
                <Button
                  label={daemon().name}
                  variant="secondary"
                  testID="daemon-row"
                  onPress={() => selectDaemon(daemon())}
                />
              )}
            </Each>
          </If>
        }
      >
        {(message) => <Text class="text-sm text-destructive">{message()}</Text>}
      </If>
    </Card>
  );
}

function KeyForm() {
  const notice = () => {
    const s = session();
    return s.kind === "unpaired" ? s.notice : null;
  };
  const daemon = () => {
    const s = session();
    return s.kind === "unpaired" ? s.daemon : null;
  };
  const submit = () => submitKey(keyDraft());
  return (
    <Card>
      <View class="gap-0.5">
        <Text class="text-base font-semibold text-foreground">{daemon()?.name ?? ""}</Text>
        <Text class="text-xs text-muted-foreground">{daemon()?.url ?? ""}</Text>
      </View>
      <If when={notice()}>
        {(n) => (
          <View testID="pair-notice" class="rounded-md bg-destructive/10 px-3 py-2">
            <Text class="text-sm text-destructive">{n().message}</Text>
          </View>
        )}
      </If>
      <TextField
        label="Pairing Key"
        testID="pairing-key"
        value={keyDraft()}
        onChange={(value) => keyDraft.set(value)}
        onSubmit={submit}
        hint="在 Daemon 的配置里查看或修改。每台设备只需配对一次。"
      />
      <Button label="配对" testID="pairing-confirm" onPress={submit} />
    </Card>
  );
}
