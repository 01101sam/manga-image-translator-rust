/** web/host.js 里导出给测试用的纯函数 (宿主本身是 createHost, 由 NSH 的网页运行时加载)。 */
export function fingerprintGrid(rgba: Uint8Array | Uint8ClampedArray, width: number, height: number): Uint8Array;
export function fingerprintHash(grid: Uint8Array): string;
export function fingerprintHex(rgba: Uint8Array | Uint8ClampedArray, width: number, height: number): string;
export function sha256Hex(bytes: Uint8Array): string;
export function createHost(player: unknown): Promise<unknown>;