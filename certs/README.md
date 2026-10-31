# 域名证书（唯一需要手工配置的密钥材料）

把证书放在本目录，文件名必须是：

```text
certs/fullchain.pem   # 证书链（服务器证书 + 中间证书）
certs/privkey.pem     # 私钥
```

然后确保：

1. `.env` 里的 `SERVER_NAME` 是证书对应的真实域名（不是 `example.com` 占位符）；
2. `./compose-up.sh` 运行（它会自动检测证书并叠加 `docker-compose.tls.yml`，
   把 80 跳转到 443）；也可以手动：
   ```bash
   docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d
   ```
3. 宿主机 80/443 没有被其他服务占用。

证书不放在镜像里，也不由脚本生成；续期后替换这两个文件并重启 `bee-admin-web`
即可（`docker compose restart bee-admin-web`）。

`certs/*.pem` 已被 `.gitignore` 忽略，不要提交私钥。
