// Compatibility bindings for the MyGo catalog and stream exports.
const storage = JSON.parse(__storage);
const settings = JSON.parse(__settings);
const translations = JSON.parse(__translations);
globalThis.host = Object.freeze({
    platform: __platform,
    arch: __arch,
    dir: typeof __pluginDir === 'string' ? __pluginDir : '',
    dataDir: typeof __dataDir === 'string' ? __dataDir : '',
    removeDataDir: () => {if (typeof __removeDataDir !== 'function' || !__removeDataDir()) throw new Error('Source data could not be removed');},
    language: () => __language,
    t: text => Object.hasOwn(translations, text) ? translations[text] : text,
    setting: key => Object.hasOwn(settings, key) ? settings[key] : "",
    spawn: (program, args) => {
        if (typeof __processSpawn === 'function') {
            const result=JSON.parse(__processSpawn(JSON.stringify({program,args})));
            if(result.error) throw new Error(result.error);
            const op=(action,text)=>{
                const value=JSON.parse(__processOp(JSON.stringify({handle:result.handle,op:action,text})));
                if(value.error)throw new Error(value.error);
                return value;
            };
            const poll=async(action,key)=>{for(;;){const value=op(action);if(!value.pending)return value[key];await new Promise(resolve=>setTimeout(resolve,10));}};
            return Object.freeze({pid:result.pid,write:text=>{op('write',String(text));},closeStdin:()=>{op('close');},readLine:()=>poll('read','line'),wait:()=>poll('wait','status'),errorOutput:()=>op('stderr').text,kill:()=>{op('kill');}});
        }
        if (typeof __spawn !== "function") throw new Error("External helper programs are not available for this operation");
        const result = JSON.parse(__spawn(JSON.stringify({ program, args })));
        if (result.error) throw new Error(result.error);
        // The bounded host operation has completed before the handle is returned.
        return Object.freeze({
            closeStdin() {},
            readLine: async () => null,
            wait: async () => result.status ?? 0,
            errorOutput: () => result.stderr || "",
            kill() {},
        });
    },
    openURL: url => {
        const result=JSON.parse(__openURL(String(url)));
        if(result.error) throw new Error(result.error);
    },
    waitForRedirect: async options => {
        const start=JSON.parse(__redirectStart(JSON.stringify(options)));
        if(start.error) throw new Error(start.error);
        for (;;) {
            const result=JSON.parse(__redirectPoll());
            if(result.error) throw new Error(result.error);
            if(!result.pending) return result.value;
            await new Promise(resolve=>setTimeout(resolve,50));
        }
    },
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
            const next={...storage};
            Object.defineProperty(next,key,{value,enumerable:true,configurable:true});
            if (!__save(JSON.stringify(next))) throw new Error("Plugin storage write failed");
            Object.defineProperty(storage, key, { value, enumerable: true, configurable: true });
        },
        remove: key => {
            const next={...storage}; delete next[key];
            if (!__save(JSON.stringify(next))) throw new Error("Plugin storage write failed");
            delete storage[key];
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
globalThis.__invoke = async (fn, args) => {
    try { return {ok:true,value:await fn(args)}; }
    catch(error) { return {ok:false,code:String(error?.code||""),message:String(error?.message||error||"Plugin failed")}; }
};
const base64Alphabet='ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
globalThis.btoa=value=>{
    const text=String(value);let result='';
    for(let i=0;i<text.length;i+=3){
        const a=text.charCodeAt(i),b=text.charCodeAt(i+1),c=text.charCodeAt(i+2);
        if(a>255||b>255||c>255)throw new TypeError('btoa expects byte characters');
        const n=(a<<16)|((b||0)<<8)|(c||0);
        result+=base64Alphabet[n>>>18&63]+base64Alphabet[n>>>12&63]+(i+1<text.length?base64Alphabet[n>>>6&63]:'=')+(i+2<text.length?base64Alphabet[n&63]:'=');
    }return result;
};
globalThis.atob=value=>{
    let text=String(value).replace(/[\t\n\f\r ]/g,'');
    if(text.length%4===0)text=text.replace(/={1,2}$/,'');
    if(text.length%4===1||/[^A-Za-z0-9+/]/.test(text))throw new TypeError('Invalid base64');
    let bits=0,n=0,result='';for(const ch of text){n=(n<<6)|base64Alphabet.indexOf(ch);bits+=6;if(bits>=8){bits-=8;result+=String.fromCharCode(n>>>bits&255);}}return result;
};
globalThis.fetch = async (url, init = {}) => {
    if (init.signal?.aborted) throw new Error("Request aborted");
    const requestHeaders = new Headers(init.headers);
    let body = init.body,bodyBase64;
    if (body instanceof URLSearchParams) {
        body=body.toString();
        if (!requestHeaders.has("content-type")) requestHeaders.set("content-type","application/x-www-form-urlencoded;charset=UTF-8");
    } else if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) {
        const bytes=body instanceof ArrayBuffer?new Uint8Array(body):new Uint8Array(body.buffer,body.byteOffset,body.byteLength);
        let binary='';for(const byte of bytes)binary+=String.fromCharCode(byte);bodyBase64=btoa(binary);body=undefined;
    } else if (body != null) { body=String(body); }
    const request = JSON.stringify({
        url: String(url), method: init.method, headers: Object.fromEntries(requestHeaders), body,bodyBase64
    });
    let raw;
    if (typeof __fetchStart === "function") {
        const started=JSON.parse(__fetchStart(request));
        if (started.error) throw new Error(started.error);
        for (;;) {
            if (init.signal?.aborted) throw new Error("Request aborted");
            raw=__fetchPoll(started.id);
            if (raw!=null) break;
            await new Promise(resolve=>setTimeout(resolve,10));
        }
    } else { raw=__fetch(request); }
    const response = JSON.parse(raw);
    if (response.error) throw new Error(response.error);
    const headers = response.headers || [];
    return { ok: response.status >= 200 && response.status < 300, status: response.status,
        headers: new Headers(headers),
        text: async () => response.body, json: async () => JSON.parse(response.body),
        arrayBuffer: async () => response.base64 == null ? new TextEncoder().encode(response.body).buffer : Uint8Array.from(atob(response.base64),c=>c.charCodeAt(0)).buffer };
};
