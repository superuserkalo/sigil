// Vercel Routing Middleware: serve the Markdown twin of a page when the client asks for it with
// `Accept: text/markdown` (acceptmarkdown.com), and a Markdown 404 for a page that does not exist.
// Static files win over vercel.json rewrites, so this cannot be done there. Browsers never send
// that header, so pages are untouched for people.
export const config = {
  // Pages only: no file extensions, no Vercel internals.
  matcher: ['/((?!_vercel/|img/|fonts/|.*\\.[A-Za-z0-9]+$).*)'],
};

const HEADERS = {
  'content-type': 'text/markdown; charset=utf-8',
  'cache-control': 'public, max-age=0, must-revalidate',
  vary: 'Accept',
};

export default async function middleware(request) {
  const accept = request.headers.get('accept') || '';
  if (!/text\/markdown/i.test(accept)) return;
  const url = new URL(request.url);
  const path = url.pathname.replace(/\/+$/, '');
  if (!/^(\/[a-z0-9-]+)?$/.test(path)) return;
  const twin = new URL((path === '' ? '/index' : path) + '.md', url.origin);
  const found = await fetch(twin, { headers: { accept: 'text/plain' } });
  if (found.ok) return new Response(await found.text(), { status: 200, headers: HEADERS });
  const body = [
    '# Page not found',
    '',
    `There is no page at ${url.pathname} on this site.`,
    '',
    `- [All pages, with one line each](${url.origin}/llms.txt)`,
    `- [The whole site as one Markdown file](${url.origin}/llms-full.txt)`,
    `- [Sitemap](${url.origin}/sitemap.xml)`,
    `- [Home](${url.origin}/index.md)`,
    '',
  ].join('\n');
  return new Response(body, { status: 404, headers: HEADERS });
}
