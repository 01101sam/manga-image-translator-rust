import { describe, expect, test } from "nsh/test";
import { mediaUrl, rawMediaUrl, THUMB_WIDTH, VIEW_WIDTH } from "../src/domain/media";

describe("media URLs", () => {
  test("list thumbnails and the viewer ask for a bounded size and carry the token in the query", () => {
    expect(mediaUrl("http://h:1", "j1", "artifact", THUMB_WIDTH, "t k")).toBe("http://h:1/jobs/j1/artifact?w=256&token=t%20k");
    expect(mediaUrl("http://h:1", "j1", "source", VIEW_WIDTH, "t")).toBe("http://h:1/jobs/j1/source?w=2048&token=t");
    expect([256, 512, 1024, 2048]).toContain(THUMB_WIDTH);
    expect([256, 512, 1024, 2048]).toContain(VIEW_WIDTH);
  });

  test("the raw URL for opening non-image artifacts in a browser has no w", () => {
    expect(rawMediaUrl("http://h:1", "j1", "artifact", "t")).toBe("http://h:1/jobs/j1/artifact?token=t");
  });
});
