export function sitemap(site: URL): Response {
  const paths = ['/', '/zh/'];
  const alternates = [
    ['en', '/'],
    ['zh-CN', '/zh/'],
    ['x-default', '/'],
  ] as const;
  const links = alternates.map(([language, path]) =>
    `<xhtml:link rel="alternate" hreflang="${language}" href="${new URL(path, site).href}"/>`,
  ).join('');
  return new Response(`<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">${paths.map((path) => `<url><loc>${new URL(path, site).href}</loc>${links}</url>`).join('')}</urlset>`, {
    headers: { 'Content-Type': 'application/xml; charset=utf-8' },
  });
}

export function robots(site: URL): Response {
  return new Response(`User-agent: *\nAllow: /\n\nSitemap: ${new URL('/sitemap.xml', site).href}\n`, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
