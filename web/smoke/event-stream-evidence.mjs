// Observe delivered bytes without replacing the application's fetch or reader.
export async function observeEventStreams(page, suffix, starts, ends) {
  const session = await page.context().newCDPSession(page);
  const requests = new Map();
  session.on('Network.requestWillBeSent', ({ requestId, request }) => {
    if (new URL(request.url).pathname !== suffix) return;
    const evidence = { at: new Date().toISOString(), sequence: starts.length + 1 };
    starts.push(evidence);
    requests.set(requestId, { evidence, text: '', decoder: new TextDecoder() });
  });
  const receive = (state, data) => {
    if (!data || state.evidence.confirmedAt) return;
    state.text += state.decoder.decode(Buffer.from(data, 'base64'), { stream: true });
    const frames = state.text.split(/\r\n\r\n|\n\n|\r\r/);
    state.text = frames.pop().slice(-65536);
    if (frames.some(frame => /^(?:data:|:)/m.test(frame))) {
      state.evidence.confirmedAt = new Date().toISOString();
      state.text = '';
    }
  };
  session.on('Network.responseReceived', async ({ requestId, response }) => {
    const state = requests.get(requestId);
    if (!state) return;
    state.evidence.status = response.status;
    const contentType = Object.entries(response.headers).find(([name]) => name.toLowerCase() === 'content-type')?.[1];
    if (response.status !== 200 || contentType?.split(';')[0].trim().toLowerCase() !== 'text/event-stream') return;
    state.accepted = true;
    try {
      const { bufferedData } = await session.send('Network.streamResourceContent', { requestId });
      // Events can arrive while the command is pending. Preserve their order.
      receive(state, bufferedData);
      for (const data of state.pending ?? []) receive(state, data);
      state.pending = null;
      state.streaming = true;
    } catch {
      state.evidence.observationFailed = true;
    }
  });
  session.on('Network.dataReceived', ({ requestId, data }) => {
    const state = requests.get(requestId);
    if (!state?.accepted || !data) return;
    if (state.streaming) receive(state, data);
    else (state.pending ??= []).push(data);
  });
  const end = ({ requestId }, outcome) => {
    const state = requests.get(requestId);
    if (!state) return;
    ends.push({ at: new Date().toISOString(), sequence: state.evidence.sequence, outcome, status: state.evidence.status });
    requests.delete(requestId);
  };
  session.on('Network.loadingFinished', event => end(event, 'finished'));
  session.on('Network.loadingFailed', event => end(event, 'failed'));
  await session.send('Network.enable');
}
