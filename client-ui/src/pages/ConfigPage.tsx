import { Scroll, Text, View } from "nsh";
import { CONFIG_SECTIONS, fieldId, getIn, type FieldSpec } from "../domain/config";
import {
  configDirty,
  configDraft,
  configError,
  configErrors,
  configLoad,
  configSaving,
  discardConfig,
  editConfig,
  loadConfig,
  saveConfig,
} from "../store/config";
import { changeDaemon } from "../store/app";
import { engineBusy, engineLabel, engineState, runEngineAction } from "../store/engine";
import { session } from "../store/session";
import { Badge, Button, Card, NumberField, SelectField, Spinner, SwitchField, TextField, type BadgeTone } from "../ui";

const engineTone = (): BadgeTone => {
  switch (engineState()) {
    case "running":
      return "success";
    case "starting":
    case "draining":
      return "warning";
    default:
      return "neutral";
  }
};

export function ConfigPage() {
  void loadConfig();
  return (
    <View class="flex-1 bg-background">
      <Scroll class="flex-1" contentClass="w-full max-w-3xl gap-4 self-center p-4 pb-8">
        <Text class="text-xl font-semibold text-foreground">配置</Text>
        <EngineCard />
        <If when={configLoad() === "loading"}>
          <Card class="items-center py-8">
            <Spinner label="正在读取配置…" />
          </Card>
        </If>
        <If when={configLoad() === "failed"}>
          <Card>
            <Text testID="config-error" class="text-sm text-destructive">{configError() ?? "读取配置失败"}</Text>
            <Button label="重试" variant="secondary" onPress={() => void loadConfig()} />
          </Card>
        </If>
        <If when={configLoad() === "ready"}>
          <Each of={CONFIG_SECTIONS} key={(section) => section.id}>
            {(section) => (
              <Card>
                <Text class="text-base font-semibold text-foreground">{section().title}</Text>
                <Each of={section().fields} key={(field) => fieldId(field)}>
                  {(field) => <ConfigField spec={field()} />}
                </Each>
              </Card>
            )}
          </Each>
        </If>
        <Card>
          <Text class="text-base font-semibold text-foreground">连接</Text>
          <Text class="text-sm text-muted-foreground">{connectionText()}</Text>
          <Button label="取消配对" variant="secondary" testID="forget" onPress={changeDaemon} />
        </Card>
      </Scroll>
      <If when={configDirty()}>
        <View testID="config-savebar" class="flex-row items-center justify-end gap-2 border-t border-border bg-card px-4 py-3 pb-[env(safe-area-inset-bottom)]">
          <Text class="flex-1 text-sm text-muted-foreground">有未保存的修改</Text>
          <Button label="放弃" variant="ghost" onPress={discardConfig} />
          <Button label="保存" testID="config-save" loading={configSaving()} onPress={() => void saveConfig()} />
        </View>
      </If>
    </View>
  );
}

function connectionText(): string {
  const s = session();
  return s.kind === "paired" ? `已连接 ${s.daemon.name} (${s.daemon.url})。Token 保存在本设备, 之后无需再次配对。` : "";
}

function EngineCard() {
  return (
    <Card>
      <View class="flex-row items-center justify-between">
        <Text class="text-base font-semibold text-foreground">Engine</Text>
        <Badge testID="engine-state" label={engineLabel(engineState())} tone={engineTone()} />
      </View>
      <Text class="text-sm text-muted-foreground">Engine 负责加载模型并翻译。关闭后 Daemon 仍在运行, 可以随时启动。</Text>
      <View class="flex-row flex-wrap gap-2">
        <Button
          label="启动"
          icon="play"
          testID="engine-start"
          disabled={engineBusy() || engineState() === "running" || engineState() === "starting"}
          onPress={() => void runEngineAction("start")}
        />
        <Button
          label="关闭"
          icon="stop"
          variant="secondary"
          testID="engine-stop"
          disabled={engineBusy() || engineState() === "stopped"}
          onPress={() => void runEngineAction("stop")}
        />
        <Button label="重启" icon="refresh" variant="secondary" testID="engine-restart" disabled={engineBusy()} onPress={() => void runEngineAction("restart")} />
      </View>
    </Card>
  );
}

/** 按字段表渲染一项。文档里没有的字段 (Daemon 版本较旧) 不显示。 */
function ConfigField(props: { spec: FieldSpec }) {
  const value = () => getIn(configDraft(), props.spec.path);
  const error = () => configErrors().get(fieldId(props.spec)) ?? null;
  const set = (next: unknown) => editConfig(props.spec.path, next);
  const kind = props.spec.kind;
  return (
    <If when={value() !== undefined}>
      {kind.type === "text" ? (
        <TextField
          label={props.spec.label}
          hint={props.spec.hint}
          error={error()}
          testID={`cfg-${fieldId(props.spec)}`}
          placeholder={kind.placeholder}
          value={typeof value() === "string" ? (value() as string) : ""}
          onChange={(text) => set(kind.nullable && text === "" ? null : text)}
        />
      ) : kind.type === "secret" ? (
        <TextField
          label={props.spec.label}
          hint={props.spec.hint}
          error={error()}
          secret
          testID={`cfg-${fieldId(props.spec)}`}
          value={typeof value() === "string" ? (value() as string) : ""}
          onChange={(text) => set(text === "" ? null : text)}
        />
      ) : kind.type === "int" || kind.type === "float" ? (
        <NumberField
          label={props.spec.label}
          hint={props.spec.hint}
          error={error()}
          testID={`cfg-${fieldId(props.spec)}`}
          value={typeof value() === "number" ? (value() as number) : 0}
          onChange={(number) => set(number)}
        />
      ) : kind.type === "optional-number" ? (
        <View class="gap-2">
          <SwitchField
            label={`${props.spec.label}: 自定义`}
            hint={props.spec.hint}
            checked={value() !== null}
            testID={`cfg-${fieldId(props.spec)}-on`}
            onChange={(on) => set(on ? kind.fallback : null)}
          />
          <If when={value() !== null}>
            <NumberField
              label={props.spec.label}
              error={error()}
              testID={`cfg-${fieldId(props.spec)}`}
              value={typeof value() === "number" ? (value() as number) : kind.fallback}
              onChange={(number) => set(number)}
            />
          </If>
        </View>
      ) : kind.type === "switch" ? (
        <SwitchField
          label={props.spec.label}
          hint={props.spec.hint}
          checked={value() === true}
          testID={`cfg-${fieldId(props.spec)}`}
          onChange={(checked) => set(checked)}
        />
      ) : (
        <SelectField
          label={props.spec.label}
          hint={props.spec.hint}
          testID={`cfg-${fieldId(props.spec)}`}
          value={typeof value() === "string" ? (value() as string) : null}
          options={kind.options}
          emptyLabel={kind.nullable ? "不使用" : undefined}
          onChange={(next) => set(next)}
        />
      )}
    </If>
  );
}
