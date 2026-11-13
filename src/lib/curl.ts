import type { NetworkRequest } from '../types'

/**
 * Headers that must not be copied verbatim:
 * - host / content-length / transfer-encoding are derived by curl itself
 * - connection / proxy-connection / proxy-authorization are hop-by-hop fields
 * - accept-encoding is dropped so curl negotiates a body it can print as-is
 */
const SKIP_HEADERS = new Set([
  'host',
  'content-length',
  'transfer-encoding',
  'connection',
  'proxy-connection',
  'proxy-authorization',
  'accept-encoding'
])

/** POSIX single-quote escaping: close the quote, escape the quote, reopen it. */
function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`
}

/**
 * Render a captured request as a cURL command.
 *
 * `CONNECT` rows are transport metadata (an undecrypted HTTPS tunnel), not HTTP
 * requests, so they return an empty string and the UI hides the button.
 * Header values are the already-redacted ones — a copied command never contains
 * a real token or cookie.
 */
export function toCurl(request: NetworkRequest): string {
  if (request.method === 'CONNECT') {
    return ''
  }
  const method = (request.method || 'GET').toUpperCase()
  const url =
    request.url ||
    (request.host ? `${request.scheme || 'http'}://${request.host}${request.path || ''}` : '')
  if (!url) {
    return ''
  }

  const lines = [`curl -X ${method} ${shellQuote(url)}`]
  for (const header of request.requestHeaders) {
    const name = header.name.trim()
    if (!name || SKIP_HEADERS.has(name.toLowerCase())) {
      continue
    }
    lines.push(`-H ${shellQuote(`${name}: ${header.value}`)}`)
  }
  if (request.requestBody) {
    // --data-raw keeps the body byte-for-byte, including JSON quotes and `$`.
    lines.push(`--data-raw ${shellQuote(request.requestBody)}`)
  }
  return lines.join(' \\\n  ')
}
