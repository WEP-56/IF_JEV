/**
 * Tauri IPC 的唯一入口。
 *
 * 浏览器预览模式下没有 `window.__TAURI__`，调用会明确失败而不是静默返回演示数据——
 * 「看起来能用其实没接上」比「报错说没接上」危险得多。
 */

declare global {
  interface Window {
    __TAURI__?: {
      core?: { invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T> };
      event?: {
        listen: <T>(event: string, handler: (payload: { payload: T }) => void) => Promise<() => void>;
      };
      window?: {
        getCurrentWindow: () => {
          minimize: () => Promise<void>;
          toggleMaximize: () => Promise<void>;
          close: () => Promise<void>;
          startDragging: () => Promise<void>;
        };
      };
    };
  }
}

/** 是否运行在 Tauri 桌面端（相对纯浏览器预览）。 */
export function isNative(): boolean {
  return Boolean(window.__TAURI__?.core?.invoke);
}

export function tauriInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return Promise.reject(new Error('此功能需要运行 Tauri 桌面应用'));
  return invoke<T>(command, args);
}
