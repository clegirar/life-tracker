# Le build ne dépend pas de l'architecture (que du JavaScript, Argon2 en WebAssembly) : il tourne en
# natif, seule l'image finale est amd64.
FROM --platform=$BUILDPLATFORM node:22.16.0-alpine@sha256:41e4389f3d988d2ed55392df4db1420ad048ae53324a8e2b7c6d19508288107e AS build
WORKDIR /src
COPY package.json package-lock.json .npmrc ./
RUN npm ci
COPY tsconfig.json tsconfig.build.json ./
COPY src ./src
RUN npm run build && npm prune --omit=dev
# Le dossier des jetons : un volume neuf hérite de ce propriétaire. Ici, car RUN dans l'image finale
# demanderait d'émuler amd64 sur un Mac.
RUN mkdir -p /mcp-state

FROM node:22.16.0-alpine@sha256:41e4389f3d988d2ed55392df4db1420ad048ae53324a8e2b7c6d19508288107e
ENV NODE_ENV=production
WORKDIR /app
COPY --from=build /src/package.json ./
COPY --from=build /src/node_modules ./node_modules
COPY --from=build /src/dist ./dist
COPY --from=build --chown=10001:10001 /mcp-state /var/lib/life-tracker-mcp
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["node", "/app/dist/main.js"]
CMD ["serve"]
