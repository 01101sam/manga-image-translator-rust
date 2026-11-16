import { parseHash } from "./similar";

/**
 * 持久化的一行: 重启或刷新后据此恢复列表。顺序就是当时的显示顺序。
 * 文件名、大小、修改时间来自导入时, 恢复后排序还要用, 所以一起存。
 * `id` 是 Job 的 id; 还没上传的文件 (待上传、上传中、被拒绝) 是 `local:<宿主的 fileId>`, 多存一个 type 和被拒绝的原因,
 * 重启后由宿主凭 fileId 找回文件 (缩略图不存: 每次存档都整份写, 几百张缩略图会让存档大到拖慢列表, 宿主自己留着)。
 */
export interface SavedJob {
  readonly id: string;
  readonly name?: string;
  readonly size?: number;
  readonly modifiedAt?: number;
  readonly type?: string;
  readonly rejected?: string;
  /** 相似图片的指纹和宽高比 (见 domain/similar.ts), 重启后新导入的图还要和已有的比。 */
  readonly hash?: string;
  readonly aspect?: number;
}

/**
 * 紧凑格式 [[id, name, size, modifiedAt], ...], 缺失的值写 null, 两千项也只有几十 KB。
 * 还没上传的文件的行多两项 [..., type, rejected]; 有指纹的行再多两项 [..., hash, aspect]; 没有这些项时行和以前一样, 老版本也读得懂。
 */
export function encodeSaved(jobs: readonly SavedJob[]): string {
  return JSON.stringify(
    jobs.map((job) => {
      const row: (string | number | null)[] = [job.id, job.name ?? null, job.size ?? null, job.modifiedAt ?? null];
      if (job.type !== undefined || job.rejected !== undefined || job.hash !== undefined) row.push(job.type ?? null, job.rejected ?? null);
      if (job.hash !== undefined) row.push(job.hash, job.aspect ?? null);
      return row;
    }),
  );
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
    const [id, name, size, modifiedAt, type, rejected, hash, aspect] = row as unknown[] & [string];
    return [
      {
        id,
        name: typeof name === "string" ? name : undefined,
        size: finite(size) && size > 0 ? size : undefined,
        modifiedAt: finite(modifiedAt) && modifiedAt > 0 ? modifiedAt : undefined,
        ...(typeof type === "string" ? { type } : {}),
        ...(typeof rejected === "string" ? { rejected } : {}),
        ...(parseHash(hash) === undefined ? {} : { hash: hash as string }),
        ...(parseHash(hash) !== undefined && finite(aspect) && aspect > 0 ? { aspect } : {}),
      },
    ];
  });
}