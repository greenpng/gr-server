# gv6 official site (官网) — lab-local 容器镜像
# 构建: docker build -t gv6-official:lab .
# 官网 lab 镜像。node_modules 从构建上下文复制 —— 本机 npm ci 在容器内
# 经代理不稳定 (npm 10 "Exit handler never called"), 且宿主 node_modules 已是
# 完整依赖 (官网为纯 JS 依赖, 无原生编译模块, alpine glibc/musl 差异无影响)。
FROM node:22-alpine
ENV NODE_ENV=production
WORKDIR /app
COPY package.json package-lock.json ./
COPY node_modules ./node_modules
COPY server ./server
COPY web ./web
EXPOSE 4101
CMD ["node", "server/src/index.js"]
