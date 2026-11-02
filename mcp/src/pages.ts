// Les deux pages du serveur : le consentement (avec le formulaire de connexion) et les refus.
// Aucune ressource externe, aucun script : la CSP de `app.ts` n'autorise que le style en ligne.

function escape(text: string): string {
  return text.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

function page(title: string, body: string): string {
  return `<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<meta name="referrer" content="no-referrer">
<title>${escape(title)}</title>
<style>
:root { --bg: #f5f5f7; --card: #fff; --text: #1d1d1f; --muted: #6e6e73; --accent: #0071e3; --danger: #d70015; }
@media (prefers-color-scheme: dark) { :root { --bg: #000; --card: #1c1c1e; --text: #f5f5f7; --muted: #98989d; --accent: #0a84ff; --danger: #ff453a; } }
* { box-sizing: border-box; }
body { margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 16px; background: var(--bg); color: var(--text);
  font: 17px/1.45 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", sans-serif; -webkit-text-size-adjust: 100%; }
main { width: 100%; max-width: 380px; background: var(--card); border-radius: 18px; padding: 28px 24px; }
h1 { font-size: 22px; line-height: 1.2; margin: 0 0 12px; letter-spacing: -0.01em; }
p { margin: 0 0 12px; color: var(--muted); }
strong { color: var(--text); font-weight: 600; }
label { display: block; font-size: 15px; margin: 20px 0 6px; }
input[type=password] { width: 100%; font-size: 17px; padding: 12px 14px; border-radius: 12px; border: 1px solid color-mix(in srgb, var(--muted) 40%, transparent);
  background: transparent; color: var(--text); }
.actions { display: flex; gap: 10px; margin-top: 20px; }
button { flex: 1; font: inherit; font-weight: 600; padding: 12px; border-radius: 12px; border: 0; cursor: pointer; }
button.primary { background: var(--accent); color: #fff; }
button.secondary { background: color-mix(in srgb, var(--muted) 18%, transparent); color: var(--text); }
.error { color: var(--danger); font-size: 15px; margin-top: 12px; }
</style>
</head>
<body><main>${body}</main></body>
</html>`;
}

export function consentPage(options: {
  requestId: string;
  csrf: string;
  redirectHost: string;
  scope: string;
  error?: string;
}): string {
  const error = options.error ? `<p class="error" role="alert">${escape(options.error)}</p>` : '';
  return page(
    'Autoriser Claude',
    `<h1>Autoriser Claude à lire le Life tracker ?</h1>
<p>Claude pourra <strong>lire</strong> les données, en lecture seule et sous pseudonymes, jusqu'à ce que tu le déconnectes.</p>
<p>Retour vers <strong>${escape(options.redirectHost)}</strong> · droits : ${escape(options.scope)}</p>
<form method="post" action="/authorize">
<input type="hidden" name="request" value="${escape(options.requestId)}">
<input type="hidden" name="csrf" value="${escape(options.csrf)}">
<label for="secret">Secret de connexion du MCP</label>
<input id="secret" name="secret" type="password" autocomplete="current-password" required autofocus>
${error}
<div class="actions">
<button class="secondary" type="submit" name="decision" value="deny" formnovalidate>Refuser</button>
<button class="primary" type="submit" name="decision" value="allow">Autoriser</button>
</div>
</form>`,
  );
}

export function messagePage(title: string, message: string): string {
  return page(title, `<h1>${escape(title)}</h1><p>${escape(message)}</p>`);
}
