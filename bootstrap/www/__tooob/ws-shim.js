// Injected into the site iframe. A cookie the Site sets for a whole domain would reach every other
// Host's pages on the Relay, so drop its Domain= and keep it with this Key Address (ADR 0021).
(() => {
  const cookie = Object.getOwnPropertyDescriptor(Document.prototype, 'cookie');
  Object.defineProperty(Document.prototype, 'cookie', {
    ...cookie,
    set(v) {
      cookie.set.call(this, String(v).replace(/;\s*domain\s*=[^;]*/gi, ''));
    },
  });
  if (!window.CookieStore) return;
  // A cookie set without its domain can only be deleted without it too.
  for (const method of ['set', 'delete']) {
    const real = CookieStore.prototype[method];
    CookieStore.prototype[method] = function (name, ...rest) {
      if (name && typeof name === 'object') {
        const { domain, ...options } = name;
        return real.call(this, options, ...rest);
      }
      return real.call(this, name, ...rest);
    };
  }
})();

// Service workers can't intercept WebSockets, so replace the constructor with one that tunnels
// through the shell page's iroh connection.
(() => {
  const shell = parent !== window && parent.__tooob;
  if (!shell) return;
  const enc = new TextEncoder();
  const dec = new TextDecoder();

  class TooobWebSocket extends EventTarget {
    constructor(url, protocols) {
      super();
      const u = new URL(url, location.href);
      this.url = u.href;
      this.readyState = 0;
      this.protocol = '';
      this.extensions = '';
      this.bufferedAmount = 0;
      this.binaryType = 'blob';
      for (const type of ['open', 'message', 'close', 'error']) {
        this['on' + type] = null;
        this.addEventListener(type, (ev) => this['on' + type]?.call(this, ev));
      }
      const fire = (ev) => setTimeout(() => this.dispatchEvent(ev));
      this._conn = shell.openWs(u.pathname + u.search, protocols == null ? [] : [].concat(protocols), {
        open: (protocol) => {
          this.readyState = 1;
          this.protocol = protocol;
          fire(new Event('open'));
        },
        message: (kind, payload) => {
          const bytes = new Uint8Array(payload); // copy into this realm
          const data = kind === 0 ? dec.decode(bytes) : this.binaryType === 'arraybuffer' ? bytes.buffer : new Blob([bytes]);
          fire(new MessageEvent('message', { data }));
        },
        close: (code, reason) => {
          if (this.readyState === 3) return;
          this.readyState = 3;
          fire(new CloseEvent('close', { code, reason, wasClean: code === 1000 }));
        },
        error: () => fire(new Event('error')),
      });
    }

    send(data) {
      if (this.readyState === 0) throw new DOMException('WebSocket is still connecting', 'InvalidStateError');
      if (typeof data === 'string') return this._conn.send(0, enc.encode(data));
      if (data instanceof Blob) return void data.arrayBuffer().then((b) => this._conn.send(1, new Uint8Array(b)));
      this._conn.send(1, ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data));
    }

    close() {
      if (this.readyState >= 2) return;
      this.readyState = 2;
      this._conn.close();
    }
  }
  Object.assign(TooobWebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  Object.assign(TooobWebSocket.prototype, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  window.WebSocket = TooobWebSocket;
})();
