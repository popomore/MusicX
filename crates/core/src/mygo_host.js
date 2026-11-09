// Compatibility bindings for the MyGo catalog and stream exports.
const storage = JSON.parse(__storage);
const settings = JSON.parse(__settings);
globalThis.host = Object.freeze({
    platform: __platform,
    arch: __arch,
    language: () => "zh-CN",
    t: text => text,
    setting: key => Object.hasOwn(settings, key) ? settings[key] : "",
    signInPrompt: prompt => {
        if (!__signInPrompt(JSON.stringify(prompt))) throw new Error("Open the source sign-in page to continue");
    },
    webSignIn: async options => {
        const result=JSON.parse(__webSignIn(JSON.stringify(options)));
        if(result.error)throw new Error(result.error);
        return result;
    },
    storage: Object.freeze({
        get: key => Object.hasOwn(storage, key) ? storage[key] : null,
        set: (key, value) => {
            Object.defineProperty(storage, key, { value, enumerable: true, configurable: true });
            if (!__save(JSON.stringify(storage))) throw new Error("Plugin storage write failed");
        },
        remove: key => {
            delete storage[key];
            if (!__save(JSON.stringify(storage))) throw new Error("Plugin storage write failed");
        },
    }),
    crypto: Object.freeze({
        md5: text => __md5(String(text)),
        aesEcbEncrypt: (key, text) => {
            const result = JSON.parse(__aesEcb(String(key), String(text)));
            if (result.error) throw new Error(result.error);
            return result.hex;
        },
    }),
});
globalThis.URL = class {
    constructor(value) {
        const result = JSON.parse(__url(String(value)));
        if (result.error) throw new TypeError(result.error);
        Object.assign(this, result);
    }
    toString() { return this.href; }
};
globalThis.URLSearchParams = class {
    #items = [];
    constructor(init = {}) {
        for (const [key, value] of Object.entries(init)) this.append(key, value);
    }
    append(key, value) { this.#items.push([String(key), String(value)]); }
    set(key, value) {
        this.#items = this.#items.filter(item => item[0] !== String(key));
        this.append(key, value);
    }
    toString() { return __query(JSON.stringify(this.#items)); }
};
globalThis.AbortController = class {
    signal = { aborted: false };
    abort() { this.signal.aborted = true; }
};
const timers = new Map();
let timerId = 0;
globalThis.setTimeout = (callback, ms, ...args) => {
    const id = ++timerId;
    timers.set(id, { due: Date.now() + Math.max(0, Number(ms) || 0), callback, args });
    return id;
};
globalThis.clearTimeout = id => timers.delete(id);
globalThis.__nextTimer = () => {
    let next;
    for (const entry of timers) {
        if (!next || entry[1].due < next[1].due) next = entry;
    }
    if (!next) return false;
    const [id, timer] = next;
    if (!__sleep(Math.max(0, timer.due - Date.now()))) throw new Error("Plugin deadline exceeded");
    timers.delete(id);
    timer.callback(...timer.args);
    return true;
};
globalThis.console = Object.freeze({ log() {}, warn() {}, error() {} });
globalThis.fetch = async (url, init = {}) => {
    if (init.signal?.aborted) throw new Error("Request aborted");
    const response = JSON.parse(__fetch(JSON.stringify({
        url: String(url), method: init.method, headers: init.headers, body: init.body
    })));
    if (response.error) throw new Error(response.error);
    const headers = response.headers || [];
    return { ok: response.status >= 200 && response.status < 300, status: response.status,
        headers: Object.freeze({
            getSetCookie: () => headers.filter(([key]) => key === "set-cookie").map(([, value]) => value),
            get: key => headers.filter(([name]) => name === String(key).toLowerCase()).map(([, value]) => value).join(", ") || null,
        }),
        text: async () => response.body, json: async () => JSON.parse(response.body) };
};
