// L'heure au format du contrat d'ingestion : RFC 3339 avec le décalage local de l'instant, pour que
// le serveur garde l'heure de Paris telle que je l'ai vécue (heure d'été comprise).

const pad = (n: number, width = 2): string => String(n).padStart(width, "0");

/** `ms` (depuis l'époque Unix) à l'heure locale décalée de `offsetMinutes` (Paris en hiver : 60). */
export function rfc3339WithOffset(ms: number, offsetMinutes: number): string {
  const local = new Date(ms + offsetMinutes * 60_000);
  const sign = offsetMinutes < 0 ? "-" : "+";
  const abs = Math.abs(offsetMinutes);
  return (
    `${pad(local.getUTCFullYear(), 4)}-${pad(local.getUTCMonth() + 1)}-${pad(local.getUTCDate())}` +
    `T${pad(local.getUTCHours())}:${pad(local.getUTCMinutes())}:${pad(local.getUTCSeconds())}` +
    `.${pad(local.getUTCMilliseconds(), 3)}${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`
  );
}

/** `ms` avec le décalage du fuseau du navigateur à cet instant-là. */
export function localRfc3339(ms: number): string {
  // getTimezoneOffset est inversé : -60 à Paris en hiver.
  return rfc3339WithOffset(ms, -new Date(ms).getTimezoneOffset());
}
