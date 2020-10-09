import { Netmask } from 'netmask';
import isIP from 'validator/lib/isIP';

const RANGES_ENDPOINT = 'https://ip-ranges.amazonaws.com/ip-ranges.json';
const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, HEAD, OPTIONS",
  "Access-Control-Max-Age": "86400",
};

var prefixes = undefined;

addEventListener('fetch', event => {
  event.respondWith(handleRequest(event.request));
})

async function handleRequest(request) {
  const req_url = new URL(request.url);
  const params = req_url.searchParams;
  const aws_lookup = params.get('ip');

  if (!aws_lookup) {
    return new Response('"ip" parameter is empty!', { status: 400, headers: CORS_HEADERS })
  }

  if (!isIP(aws_lookup)) {
    return new Response('"ip" parameter is not a valid IP address!', { status: 400, headers: CORS_HEADERS })
  }

  let cache_status = 'local';

  if (prefixes == undefined) {
    const cache = caches.default;
    let aws_rangers_response = await cache.match(RANGES_ENDPOINT);

    if (!aws_rangers_response) {
      cache_status = 'cf_miss';
      aws_rangers_response = await fetch(RANGES_ENDPOINT, { cf: { cacheEverything: true, cacheTtl: 3600 } });
    } else {
      cache_status = 'cf_hit';
    }

    const ranges_json = await aws_rangers_response.json();
    prefixes = ranges_json.prefixes.map(metadata => { return { metadata: metadata, matcher: new Netmask(metadata.ip_prefix) } });
  }

  const matches = prefixes.filter(block => block.matcher.contains(aws_lookup)).map(block => block.metadata);

  const responseJSON = {
    "requested_ip": aws_lookup,
    "cache_status": cache_status,
    "matches": matches,
  };

  return new Response(JSON.stringify(responseJSON, null, 2), { headers: CORS_HEADERS });
}
