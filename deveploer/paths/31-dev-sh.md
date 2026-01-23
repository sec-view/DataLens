## `dev.sh`（开发启动脚本）

该脚本位于仓库根目录，目标是让开发启动更稳定：

- 启动前检查端口（默认 Vite 5173）
- 可选自动杀掉占用端口的进程（默认 `FORCE_KILL=1`）
- 启动前可选清理缓存 + 预构建前端（避免“界面没更新”的错觉）
- 确保脚本退出时，子进程一起退出（通过记录并 kill process group）

---

## 使用方式

- `./dev.sh` 或 `./dev.sh tauri`
  - 检查/释放 5173
  - （可选）清理前端缓存 + `npm run build`
  - `npm run tauri`（Tauri dev；会按 `tauri.conf.json` 使用 `devPath`）
- `./dev.sh vite`
  - 只启动 `npm run dev`（Vite）

---

## 可配置环境变量

- **`FORCE_KILL=1|0`**
  - 端口被占用时是否自动 kill 占用进程（默认 1）
- **`REBUILD_FRONTEND=1|0`**
  - 启动前是否 `npm run build`（默认 1）
- **`CLEAN_FRONTEND=1|0`**
  - 启动前是否清理缓存目录（默认 1）

---

## 清理的缓存/产物目录

脚本会（best-effort）删除：

- `apps/desktop/.svelte-kit`
- `apps/desktop/build`
- `apps/desktop/dist`
- `apps/desktop/node_modules/.vite`

这些都属于可再生目录，删除后会在下次构建时恢复。

---

## 常见问题

- **`vite: command not found` / `svelte-kit: command not found`**
  - 通常是 devDependencies 没装（例如设置了 `NODE_ENV=production`）
  - 解决：在 `apps/desktop/` 运行 `npm install --include=dev`

