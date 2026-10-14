# Cloudflare Tunnel 插件

`Cloudflare Tunnel` 插件只管理本机的 `cloudflared` 进程和 Tunnel 配置。它不提供本地 HTTP 路由、正向代理或 Cloudflare API 管理功能。

## 模式

### Cloudflare 后台托管

适合已在 Cloudflare Zero Trust 后台管理 Public Hostname 的 named Tunnel。

1. 在 Cloudflare 后台创建或选择 named Tunnel，并获取 Tunnel Token。
2. 新建 Tunnel，选择“Cloudflare 后台托管”。
3. 填入 Token，保存并启动。
4. 在 Cloudflare 后台为该 Tunnel 添加多个 Public Hostname 和对应服务地址。

Token 只由宿主的 Windows DPAPI 加密保存，不会写入普通状态文件、生成的 YAML 或命令行参数。

### 本地 config.yaml

适合希望在本机维护 ordered ingress 规则的 named Tunnel。

1. 使用现有 named Tunnel 的 ID 或名称，并准备该 Tunnel 的 credentials JSON 文件。
2. 新建 Tunnel，选择“本地 config.yaml”。
3. 填写 Tunnel ID、选择 credentials JSON 和添加 ingress 规则。
4. 保存后使用“验证配置”，通过后启动。

插件会在自己的私有数据目录写入生成的 `config.yaml`，按界面规则顺序追加 Cloudflare 要求的 `http_status:404` 兜底规则。credentials JSON 保持在用户选择的原路径，插件不会复制或上传它。

## Ingress 规则

每条规则包含：

- 域名：例如 `app.example.com`，不带 `http://` 或 `https://`。
- 路径：可选，例如 `/api`。
- 服务：包含协议的本地地址，例如 `http://127.0.0.1:3000`、`https://127.0.0.1:8443` 或 `tcp://127.0.0.1:25565`。

规则按列表从上到下匹配。使用上移、下移调整优先级。

旧 Gateway 插件的数据不会自动导入。首次使用时，请在 Cloudflare Tunnel 插件中重新配置 Tunnel 和 Token。
