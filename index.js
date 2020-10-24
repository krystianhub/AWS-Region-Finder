import { isInSubnet, isIP, isIPv4 } from 'is-in-subnet'

const CF_CACHE_STATUS_HEADER = 'cf-cache-status'
const RANGES_ENDPOINT = 'https://ip-ranges.amazonaws.com/ip-ranges.json'
const CORS_HEADERS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS',
  'Access-Control-Max-Age': '86400',
}

var ipv4_prefixes = undefined
var ipv6_prefixes = undefined

addEventListener('fetch', (event) => {
  event.respondWith(handleRequest(event.request))
})

async function handleRequest(request) {
  const req_url = new URL(request.url)
  const params = req_url.searchParams
  const ip_lookup = params.get('ip')

  if (!ip_lookup) {
    return new Response('"ip" parameter is empty!', {
      status: 400,
      headers: CORS_HEADERS,
    })
  }

  if (!isIP(ip_lookup)) {
    return new Response('"ip" parameter is not a valid IP address!', {
      status: 400,
      headers: CORS_HEADERS,
    })
  }

  let cache_status = 'LOCAL'

  if (ipv4_prefixes == undefined || ipv6_prefixes == undefined) {
    let aws_rangers_response = await fetch(RANGES_ENDPOINT, {
      cf: { cacheEverything: true, cacheTtl: 3600 },
    })

    if (aws_rangers_response.headers.has(CF_CACHE_STATUS_HEADER)) {
      cache_status = aws_rangers_response.headers.get(CF_CACHE_STATUS_HEADER)
    } else {
      cache_status = 'N/A'
    }

    const ranges_json = await aws_rangers_response.json()
    ipv4_prefixes = ranges_json.prefixes
    ipv6_prefixes = ranges_json.ipv6_prefixes
  }

  let matches

  if (isIPv4(ip_lookup)) {
    matches = ipv4_prefixes.filter((block) =>
      isInSubnet(ip_lookup, block.ip_prefix),
    )
  } else {
    matches = ipv6_prefixes.filter((block) =>
      isInSubnet(ip_lookup, block.ipv6_prefix),
    )
  }

  const responseJSON = {
    requested_ip: ip_lookup,
    cache_status: cache_status,
    matches: matches,
  }

  return new Response(JSON.stringify(responseJSON, null, 2), {
    headers: CORS_HEADERS,
  })
}
