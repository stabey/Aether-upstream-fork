FROM node:22-alpine

ENV NODE_ENV=production \
    HOST=0.0.0.0 \
    PORT=8792 \
    AETHER_CURSOR_SDK_WORKSPACE=/var/lib/aether-cursor-sdk/workspace

WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force

COPY prompt.mjs server.mjs ./

RUN mkdir -p /var/lib/aether-cursor-sdk/workspace \
    && chown -R node:node /var/lib/aether-cursor-sdk /app

USER node

EXPOSE 8792

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD node -e "fetch('http://127.0.0.1:8792/healthz').then(r=>{if(!r.ok)process.exit(1)}).catch(()=>process.exit(1))"

CMD ["node", "server.mjs"]
