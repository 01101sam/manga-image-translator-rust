/**
 * Config 表单的字段表。表单只改它列出的字段, 其余原样保留:
 * `PUT /config` 会用默认值补全缺失字段, 所以必须整份取回、改完整份放回。
 */
export type ConfigDoc = Readonly<Record<string, unknown>>;

export type FieldKind =
  | { readonly type: "text"; readonly nullable?: boolean; readonly placeholder?: string }
  | { readonly type: "secret"; readonly nullable?: boolean }
  | { readonly type: "int"; readonly min?: number; readonly max?: number }
  | { readonly type: "float"; readonly min?: number; readonly max?: number }
  /** 数字或「未设置」(null): 例如字号默认由 Daemon 自动决定。 */
  | { readonly type: "optional-number"; readonly min?: number; readonly unsetLabel: string; readonly fallback: number }
  | { readonly type: "switch" }
  | { readonly type: "select"; readonly options: readonly SelectOption[]; readonly nullable?: boolean };

export interface SelectOption {
  readonly value: string;
  readonly label: string;
}

export interface FieldSpec {
  readonly path: readonly string[];
  readonly label: string;
  readonly hint?: string;
  readonly kind: FieldKind;
}

export interface SectionSpec {
  readonly id: string;
  readonly title: string;
  readonly fields: readonly FieldSpec[];
}

const opt = (value: string, label: string = value): SelectOption => ({ value, label });

export const LANGUAGES: readonly SelectOption[] = [
  opt("chs", "简体中文"),
  opt("cht", "繁体中文"),
  opt("en", "英文"),
  opt("ja", "日文"),
];

export const CONFIG_SECTIONS: readonly SectionSpec[] = [
  {
    id: "server",
    title: "服务",
    fields: [
      { path: ["pairing_key"], label: "Pairing Key", hint: "新设备配对时输入。修改只影响之后的配对, 已配对设备不受影响。", kind: { type: "text" } },
      { path: ["api_key"], label: "API Key", hint: "翻译服务的密钥, 只保存在 Daemon 本机。", kind: { type: "secret", nullable: true } },
      { path: ["workers"], label: "Worker 数量", hint: "重启 Engine 后生效。", kind: { type: "int", min: 1, max: 16 } },
      {
        path: ["downscale_long_edge"],
        label: "缩小长边",
        hint: "解码后长边超过它就缩小, 省内存。留空表示不缩小。",
        kind: { type: "optional-number", min: 256, unsetLabel: "不缩小", fallback: 4096 },
      },
    ],
  },
  {
    id: "translator",
    title: "翻译",
    fields: [
      {
        path: ["translator", "mode"],
        label: "翻译方式",
        kind: { type: "select", options: [opt("Llm", "LLM 翻译"), opt("Original", "按原文输出"), opt("None", "不输出文字")] },
      },
      { path: ["translator", "target"], label: "目标语言", kind: { type: "select", options: LANGUAGES } },
      { path: ["translator", "backend"], label: "接口类型", kind: { type: "select", options: [opt("openai", "OpenAI 兼容"), opt("anthropic", "Anthropic")] } },
      { path: ["translator", "base_url"], label: "接口地址", hint: "留空使用默认地址。", kind: { type: "text", nullable: true, placeholder: "https://api.deepseek.com" } },
      { path: ["translator", "model"], label: "模型", kind: { type: "text" } },
      { path: ["translator", "thinking"], label: "思考模式", kind: { type: "switch" } },
      {
        path: ["translator", "thinking_strength"],
        label: "思考强度",
        kind: { type: "select", options: [opt("low", "低"), opt("high", "高"), opt("max", "最高")] },
      },
      { path: ["translator", "web_search"], label: "联网搜索", kind: { type: "switch" } },
    ],
  },
  {
    id: "detector",
    title: "文字检测",
    fields: [
      { path: ["detector", "detector"], label: "检测器", kind: { type: "select", options: [opt("DBNet"), opt("Ctd", "CTD")] } },
      { path: ["detector", "options", "detect_size"], label: "检测尺寸", kind: { type: "int", min: 256, max: 4096 } },
      { path: ["detector", "options", "text_threshold"], label: "文字阈值", kind: { type: "float", min: 0, max: 1 } },
      { path: ["detector", "options", "box_threshold"], label: "文本框阈值", kind: { type: "float", min: 0, max: 1 } },
      { path: ["detector", "options", "unclip_ratio"], label: "扩张比例", kind: { type: "float", min: 0, max: 10 } },
    ],
  },
  {
    id: "ocr",
    title: "文字识别",
    fields: [
      {
        path: ["ocr", "ocr"],
        label: "识别模型",
        kind: { type: "select", options: [opt("Ocr48px", "48px"), opt("Ctc48px", "48px CTC"), opt("MangaOcr", "Manga OCR"), opt("Native", "系统")] },
      },
      { path: ["ocr", "min_text_length"], label: "最短文字长度", kind: { type: "int", min: 0, max: 100 } },
      { path: ["ocr", "prob"], label: "最低置信度", kind: { type: "float", min: 0, max: 1 } },
      { path: ["ocr", "ignore_bubble"], label: "忽略非气泡文字", hint: "1 到 50, 0 为关闭。", kind: { type: "int", min: 0, max: 50 } },
    ],
  },
  {
    id: "inpaint",
    title: "擦字",
    fields: [
      {
        path: ["inpainter", "inpainter"],
        label: "擦字模型",
        kind: {
          type: "select",
          options: [opt("LamaLarge", "Lama Large"), opt("LamaMpe", "Lama MPE"), opt("LamaAot", "Lama AOT"), opt("Original", "保留原图"), opt("None", "不擦字")],
        },
      },
      { path: ["inpainter", "inpainting_size"], label: "擦字尺寸", hint: "过大可能内存不足。", kind: { type: "int", min: 256, max: 4096 } },
      { path: ["mask_refinement", "dilation_offset"], label: "遮罩扩张", kind: { type: "float", min: 0, max: 100 } },
      { path: ["mask_refinement", "kernel_size"], label: "擦除核大小", kind: { type: "int", min: 1, max: 31 } },
      { path: ["mask_refinement", "furigana"], label: "覆盖注音", kind: { type: "switch" } },
    ],
  },
  {
    id: "render",
    title: "渲染",
    fields: [
      { path: ["render", "renderer"], label: "输出格式", hint: "只有 PNG 能在查看器里预览。", kind: { type: "select", options: [opt("Png", "PNG 图片"), opt("Html", "HTML"), opt("Raw", "原始数据")] } },
      { path: ["render", "alignment"], label: "对齐", kind: { type: "select", options: [opt("Auto", "自动"), opt("Left", "左"), opt("Center", "居中"), opt("Right", "右")] } },
      { path: ["render", "direction"], label: "排版方向", kind: { type: "select", options: [opt("Auto", "自动"), opt("Horizontal", "横排"), opt("Vertical", "竖排")] } },
      { path: ["render", "font_size"], label: "字号", kind: { type: "optional-number", min: 1, unsetLabel: "自动", fallback: 24 } },
      { path: ["render", "font_size_offset"], label: "字号偏移", kind: { type: "float", min: -100, max: 100 } },
      { path: ["render", "line_spacing"], label: "行距", kind: { type: "optional-number", min: 0, unsetLabel: "自动", fallback: 1 } },
      { path: ["render", "disable_font_border"], label: "关闭文字描边", kind: { type: "switch" } },
    ],
  },
  {
    id: "extras",
    title: "其他",
    fields: [
      {
        path: ["upscaler", "upscaler"],
        label: "超分模型",
        kind: {
          type: "select",
          nullable: true,
          options: [opt("Esrgan2x", "ESRGAN 2x"), opt("Esrgan4x", "ESRGAN 4x"), opt("EsrganAnime4x", "ESRGAN 动漫 4x"), opt("Anime4k", "Anime4K")],
        },
      },
      { path: ["tagger", "tagger"], label: "图片标签", hint: "给 LLM 翻译提供画面信息。", kind: { type: "select", nullable: true, options: [opt("Pixai", "PixAI")] } },
    ],
  },
];

export function getIn(doc: unknown, path: readonly string[]): unknown {
  let cur: unknown = doc;
  for (const key of path) {
    if (typeof cur !== "object" || cur === null) return undefined;
    cur = (cur as Record<string, unknown>)[key];
  }
  return cur;
}

/** 返回把 `path` 处设为 `value` 的新文档; 只复制路径上的对象, 其余共享。 */
export function setIn(doc: ConfigDoc, path: readonly string[], value: unknown): ConfigDoc {
  const [head, ...rest] = path;
  if (head === undefined) return doc;
  const current = doc[head];
  const child =
    rest.length === 0
      ? value
      : setIn(typeof current === "object" && current !== null ? (current as ConfigDoc) : {}, rest, value);
  return { ...doc, [head]: child };
}

/** 数字字段保存前的校验与夹取。返回错误文案, 合法时为 null。 */
export function validateField(spec: FieldSpec, value: unknown): string | null {
  const { kind } = spec;
  switch (kind.type) {
    case "text":
      if (!kind.nullable && (typeof value !== "string" || value.trim() === "")) return `${spec.label}不能为空`;
      return null;
    case "int":
    case "float": {
      if (typeof value !== "number" || !Number.isFinite(value)) return `${spec.label}需要是数字`;
      if (kind.type === "int" && !Number.isInteger(value)) return `${spec.label}需要是整数`;
      if (kind.min !== undefined && value < kind.min) return `${spec.label}不能小于 ${kind.min}`;
      if (kind.max !== undefined && value > kind.max) return `${spec.label}不能大于 ${kind.max}`;
      return null;
    }
    case "optional-number":
      if (value === null) return null;
      if (typeof value !== "number" || !Number.isFinite(value)) return `${spec.label}需要是数字`;
      if (kind.min !== undefined && value < kind.min) return `${spec.label}不能小于 ${kind.min}`;
      return null;
    default:
      return null;
  }
}

export function validateConfig(doc: ConfigDoc): Map<string, string> {
  const errors = new Map<string, string>();
  for (const section of CONFIG_SECTIONS) {
    for (const field of section.fields) {
      const value = getIn(doc, field.path);
      // 文档里没有这个字段 (Daemon 版本较旧) 时不校验也不显示。
      if (value === undefined) continue;
      const error = validateField(field, value);
      if (error) errors.set(field.path.join("."), error);
    }
  }
  return errors;
}

export const fieldId = (spec: FieldSpec): string => spec.path.join(".");

/** 把输入框里的文字解析成数字; 空串或非法返回 NaN, 交给 validateField 报错。 */
export function parseNumber(text: string): number {
  const trimmed = text.trim();
  return trimmed === "" ? Number.NaN : Number(trimmed);
}

export function isDirty(original: ConfigDoc | null, draft: ConfigDoc | null): boolean {
  if (original === null || draft === null) return false;
  return JSON.stringify(original) !== JSON.stringify(draft);
}
