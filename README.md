# AWS Region Finder

Simple API service used for information retrieval about AWS resource by its public IP address.

API service was built on a serverless platform called [Cloudflare Workers](https://workers.cloudflare.com).

## How does it work

Service simply uses the latest information provided by AWS. More specifically, it uses the following [AWS API call](https://ip-ranges.amazonaws.com/ip-ranges.json) behind the scenes. That file / API response contains all CIDR notations used by AWS services. However, sometimes it is quite hard to locate exact entry corresponding to the IP address you are looking for. That is why I created this simple service to do it for you.

## How to use it

You just need to pass **"ip"** parameter of the AWS IP address you are looking for.

In response you will get a set of **matches** (if it is a valid AWS IP address), including its CIDR group, region, and a service associated with it.

Additionally, there is a field called **"requested_ip"** which is simply echoing the IP address you are trying to look up.

**"cache_status"** is simply an information about its cached CIDR datastore (**ip-ranges.json** file). Values can be either **"local"** _(meaning it is stored in RAM)_, or **"cf_miss"** / **"cf_hit"** - both correspond to the [Cloudflare's "CF-Cache-Status" mechanics](https://support.cloudflare.com/hc/en-us/articles/200172516-Understanding-Cloudflare-s-CDN).

## Web UI

Web UI is available [here](https://aws-ui.home-cloud.workers.dev)

[![Screenshot of the Web UI's example results](./assets/web_ui.png)](https://aws-ui.home-cloud.workers.dev)

## cURL example

```bash
curl "https://aws.home-cloud.workers.dev/?ip=52.1.1.1"
```

```json
{
  "requested_ip": "52.1.1.1",
  "cache_status": "local",
  "matches": [
    {
      "ip_prefix": "52.0.0.0/15",
      "region": "us-east-1",
      "service": "AMAZON",
      "network_border_group": "us-east-1"
    },
    {
      "ip_prefix": "52.0.0.0/15",
      "region": "us-east-1",
      "service": "EC2",
      "network_border_group": "us-east-1"
    }
  ]
}
```
