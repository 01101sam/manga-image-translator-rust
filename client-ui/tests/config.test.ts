import { describe, expect, test } from "nsh/test";
import { CONFIG_SECTIONS, getIn, isDirty, setIn, validateConfig, type ConfigDoc } from "../src/domain/config";

const doc: ConfigDoc = {
  workers: 2,
  api_key: null,
  pairing_key: "sam01101",
  downscale_long_edge: 4096,
  translator: { mode: "Llm", target: "chs", model: "deepseek-v4-flash", thinking: true, extra_unknown: 7 },
  render: { font_size: null, font_size_offset: 0, line_spacing: null },
  detector: { detector: "DBNet", options: { detect_size: 2048, text_threshold: 0.5, box_threshold: 0.7, unclip_ratio: 2.3 } },
};

describe("Config form", () => {
  test("setIn changes one field and shares everything else", () => {
    const next = setIn(doc, ["translator", "target"], "ja");
    expect(getIn(next, ["translator", "target"])).toBe("ja");
    expect(getIn(next, ["translator", "extra_unknown"])).toBe(7);
    expect(next.render).toBe(doc.render);
    expect(getIn(doc, ["translator", "target"])).toBe("chs");
  });

  test("fields the form does not know survive an edit, so PUT never resets them", () => {
    const next = setIn(setIn(doc, ["workers"], 3), ["translator", "mode"], "None");
    expect(JSON.parse(JSON.stringify(next)).translator.extra_unknown).toBe(7);
  });

  test("dirty tracking", () => {
    expect(isDirty(doc, doc)).toBe(false);
    expect(isDirty(doc, setIn(doc, ["workers"], 4))).toBe(true);
    expect(isDirty(null, doc)).toBe(false);
  });

  test("validation reports the field that is wrong, in Chinese", () => {
    const bad = setIn(setIn(doc, ["workers"], 0), ["pairing_key"], "  ");
    const errors = validateConfig(bad);
    expect(errors.get("workers")).toContain("不能小于 1");
    expect(errors.get("pairing_key")).toContain("不能为空");
    expect(validateConfig(doc).size).toBe(0);
  });

  test("a non-numeric entry is an error, not a silent zero", () => {
    expect(validateConfig(setIn(doc, ["workers"], Number.NaN)).get("workers")).toContain("需要是数字");
  });

  test("null stays valid for optional numbers (auto font size, no downscale)", () => {
    expect(validateConfig(setIn(doc, ["downscale_long_edge"], null)).size).toBe(0);
    expect(validateConfig(setIn(doc, ["render", "font_size"], null)).size).toBe(0);
  });

  test("every field path is unique", () => {
    const ids = CONFIG_SECTIONS.flatMap((s) => s.fields.map((f) => f.path.join(".")));
    expect(new Set(ids).size).toBe(ids.length);
  });
});
