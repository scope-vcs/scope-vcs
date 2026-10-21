export default {
  node: { keepAliveTimeout: 2000, keepAliveTimeoutBuffer: 0 },
  fetch: () => new Response('ok'),
}
