# Le build de l'app ne dépend pas de l'architecture : il tourne en natif, seule l'image finale est amd64.
FROM --platform=$BUILDPLATFORM node:22.16.0-alpine@sha256:41e4389f3d988d2ed55392df4db1420ad048ae53324a8e2b7c6d19508288107e AS build
WORKDIR /src
COPY package.json package-lock.json .npmrc ./
RUN npm ci
COPY . .
RUN npm run build

FROM nginxinc/nginx-unprivileged:1.30.6-alpine@sha256:f5116ed4606f3eb015dd4a8e28cb9b351476dc0ad4bc6e9f93b19cec130cdf03
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY security-headers.conf proxy-backend.conf /etc/nginx/snippets/
COPY --from=build /src/dist /usr/share/nginx/html
