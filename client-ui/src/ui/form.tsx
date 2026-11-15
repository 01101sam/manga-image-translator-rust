import { platform, signal, Text, View, type JSX } from "nsh";
import { Input, Radio, RadioGroup, Switch } from "nsh/base";
import { IconButton } from "./button";

const desktop = platform.desktop;
const control = desktop ? "h-8 text-sm" : "h-11 text-base";

/** 标签 + 控件 + 说明 + 错误。错误和说明都是纯文字, 不靠颜色单独表意。 */
export function Field(props: { label: string; hint?: string; error?: string | null; children: JSX.Element }) {
  return (
    <View class="gap-1.5">
      <Text class="text-sm font-medium text-foreground">{props.label}</Text>
      {props.children}
      <If when={props.error}>{(message) => <Text class="text-xs text-destructive">{message()}</Text>}</If>
      <If when={props.hint && !props.error}>
        <Text class="text-xs text-muted-foreground">{props.hint}</Text>
      </If>
    </View>
  );
}

const inputClass = `flex-1 rounded-(--radius) border border-input bg-background px-3 text-foreground focus:border-ring focus-visible:ring-2 focus-visible:ring-ring ${control}`;

export function TextField(props: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  onSubmit?: () => void;
  placeholder?: string;
  hint?: string;
  error?: string | null;
  testID?: string;
  /** 密钥类字段默认遮罩, 右侧的眼睛可以显示。 */
  secret?: boolean;
  returnKeyType?: "done" | "go" | "next";
}) {
  const revealed = signal(false);
  return (
    <Field label={props.label} hint={props.hint} error={props.error}>
      <View class="flex-row items-center gap-1">
        <Input
          testID={props.testID}
          accessibilityLabel={props.label}
          value={props.value}
          onChange={({ value }) => props.onChange(value)}
          onSubmit={() => props.onSubmit?.()}
          placeholder={props.placeholder}
          masked={props.secret && !revealed()}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType={props.returnKeyType ?? "done"}
          class={inputClass}
        />
        <If when={props.secret}>
          <IconButton label={revealed() ? "隐藏" : "显示"} icon={revealed() ? "eye-off" : "eye"} onPress={() => revealed.update((v) => !v)} />
        </If>
      </View>
    </Field>
  );
}

/** 数字框: 编辑中保留用户输入的原文, 提交与失焦时才解析, 非法输入由 Field 的错误提示。 */
export function NumberField(props: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  hint?: string;
  error?: string | null;
  testID?: string;
}) {
  const text = signal<string | null>(null);
  return (
    <Field label={props.label} hint={props.hint} error={props.error}>
      <Input
        testID={props.testID}
        accessibilityLabel={props.label}
        value={text() ?? String(props.value)}
        keyboardType="decimal-pad"
        onChange={({ value }) => {
          text.set(value);
          const trimmed = value.trim();
          props.onChange(trimmed === "" ? Number.NaN : Number(trimmed));
        }}
        class={inputClass}
      />
    </Field>
  );
}

export function SwitchField(props: { label: string; checked: boolean; onChange: (checked: boolean) => void; hint?: string; testID?: string }) {
  return (
    <View class="gap-1">
      <Switch
        testID={props.testID}
        accessibilityLabel={props.label}
        checked={props.checked}
        onChange={({ checked }) => props.onChange(checked)}
        class={`flex-row items-center justify-between gap-3 ${desktop ? "min-h-8" : "min-h-11"}`}
        trackClass="h-7 w-12 rounded-full bg-muted p-0.5 transition-colors checked:bg-primary focus-visible:ring-2 focus-visible:ring-ring"
        thumbClass="size-6 rounded-full bg-background transition-transform checked:translate-x-5"
      >
        <Text class="flex-1 text-sm font-medium text-foreground">{props.label}</Text>
      </Switch>
      <If when={props.hint}>
        <Text class="text-xs text-muted-foreground">{props.hint}</Text>
      </If>
    </View>
  );
}

/**
 * 单选: 选项排成一排可换行的圆角标签。选的项只有 5 个上下, 全部摊开比弹出菜单更快, 触屏上也不用瞄小目标。
 * 不用 nsh/base 的 Select: 嵌入 iOS 的运行时默认只带 `nsh/base` 的 core 部分, 没有 Select、Menu、Sheet、Dialog。
 */
export function SelectField(props: {
  label: string;
  value: string | null;
  options: readonly { value: string; label: string }[];
  onChange: (value: string | null) => void;
  /** 允许「未设置」: 用 null 表示, 多出一个叫 emptyLabel 的选项。 */
  emptyLabel?: string;
  hint?: string;
  testID?: string;
}) {
  const entries = () => (props.emptyLabel === undefined ? props.options : [{ value: "", label: props.emptyLabel }, ...props.options]);
  const current = () => props.value ?? "";
  return (
    <Field label={props.label} hint={props.hint}>
      <RadioGroup testID={props.testID} accessibilityLabel={props.label} class="flex-row flex-wrap gap-2">
        <Each of={entries()} key={(entry) => entry.value}>
          {(entry) => (
            <Radio
              testID={`${props.testID ?? "choice"}-${entry().value || "none"}`}
              accessibilityLabel={entry().label}
              checked={current() === entry().value}
              onChange={({ checked }) => {
                if (checked) props.onChange(entry().value === "" ? null : entry().value);
              }}
              class={`group items-center justify-center rounded-full border border-input px-3.5 hover:bg-accent checked:border-primary checked:bg-primary/10 focus-visible:ring-2 focus-visible:ring-ring ${desktop ? "h-8" : "h-10"}`}
            >
              <Text class="text-sm font-medium text-foreground group-checked:text-primary">{entry().label}</Text>
            </Radio>
          )}
        </Each>
      </RadioGroup>
    </Field>
  );
}
