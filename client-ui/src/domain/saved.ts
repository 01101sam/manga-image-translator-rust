/**
 * 持久化的 Job 行: 重启或刷新后据此从 Daemon 恢复列表。顺序就是当时的显示顺序。
 * 文件名、大小、修改时间来自导入时, 恢复后排序还要用, 所以一起存。
 */
export interface SavedJob {
  readonly id: string;
  readonly name?: string;
  readonly size?: number;
  readonly modifiedAt?: number;
}

/** 紧凑格式 [[id, name, size, modifiedAt], ...], 缺失的值写 null, 两千项也只有几十 KB。 */
export function encodeSaved(jobs: readonly SavedJob[]): string {
  return JSON.stringify(jobs.map((job) => [job.id, job.name ?? null, job.size ?? null, job.modifiedAt ?? null]));
}

const finite = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);

/** 读存档。老版本存过 ["id"] 和 [["id", "name"]], 都认; 存档坏了就当没有。 */
export function decodeSaved(raw: string): SavedJob[] {
  let rows: unknown;
  try {
    rows = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(rows)) return [];
  return rows.flatMap((row): SavedJob[] => {
    if (typeof row === "string") return [{ id: row }];
    if (!Array.isArray(row) || typeof row[0] !== "string") return [];
    const [id, name, size, modifiedAt] = row as unknown[] & [string];
    return [
      {
        id,
        name: typeof name === "string" ? name : undefined,
        size: finite(size) && size > 0 ? size : undefined,
        modifiedAt: finite(modifiedAt) && modifiedAt > 0 ? modifiedAt : undefined,
      },
    ];
  });
}