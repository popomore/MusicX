// The native URL parser implements the same parsing model as the Web URL API.
globalThis.Headers = class Headers {
    #items=[];
    constructor(init=undefined) {
        if (init==null) return;
        if (typeof init[Symbol.iterator] === "function") {
            for (const pair of init) {
                const values=Array.from(pair);
                if (values.length!==2) throw new TypeError("Expected a header name/value pair");
                this.append(...values);
            }
        } else { for (const [key,value] of Object.entries(init)) this.append(key,value); }
    }
    #name(key) {
        key=String(key).toLowerCase();
        if (!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(key)) throw new TypeError("Invalid header name");
        return key;
    }
    #value(value) {
        value=String(value).trim();
        if (/[\r\n\0]/.test(value)) throw new TypeError("Invalid header value");
        return value;
    }
    append(key,value) { this.#items.push([this.#name(key),this.#value(value)]); }
    get(key) { const values=this.#items.filter(pair=>pair[0]===this.#name(key)).map(pair=>pair[1]); return values.length?values.join(", "):null; }
    has(key) { return this.#items.some(pair=>pair[0]===this.#name(key)); }
    delete(key) { key=this.#name(key); this.#items=this.#items.filter(pair=>pair[0]!==key); }
    set(key,value) { key=this.#name(key);value=this.#value(value);this.delete(key);this.#items.push([key,value]); }
    getSetCookie() { return this.#items.filter(pair=>pair[0]==="set-cookie").map(pair=>pair[1]); }
    *entries() { for (const key of [...new Set(this.#items.map(pair=>pair[0]))].sort()) yield [key,this.get(key)]; }
    *keys() { for (const [key] of this.entries()) yield key; }
    *values() { for (const [,value] of this.entries()) yield value; }
    [Symbol.iterator]() { return this.entries(); }
    forEach(callback,thisArg) { for (const [key,value] of this) callback.call(thisArg,value,key,this); }
};

globalThis.URLSearchParams = class URLSearchParams {
    #items = [];
    #changed;
    constructor(init = "") {
        if (typeof init === "string") {
            this.#items = JSON.parse(__queryParse(init.replace(/^\?/, "")));
        } else if (init != null && typeof init[Symbol.iterator] === "function") {
            for (const entry of init) {
                const pair = Array.from(entry);
                if (pair.length !== 2) throw new TypeError("Expected a name/value pair");
                this.#items.push(pair.map(String));
            }
        } else if (init != null) {
            this.#items = Object.entries(init).map(([key,value]) => [key,String(value)]);
        }
    }
    // URL keeps the same searchParams object when href or search changes.
    _connect(changed) { this.#changed = changed; }
    _replace(query) { this.#items = JSON.parse(__queryParse(query.replace(/^\?/, ""))); }
    #notify() { this.#changed?.(this.toString()); }
    get size() { return this.#items.length; }
    append(key, value) { this.#items.push([String(key),String(value)]); this.#notify(); }
    get(key) { return this.#items.find(pair => pair[0] === String(key))?.[1] ?? null; }
    getAll(key) { return this.#items.filter(pair => pair[0] === String(key)).map(pair => pair[1]); }
    has(key, value) { return this.#items.some(pair => pair[0] === String(key) && (value === undefined || pair[1] === String(value))); }
    delete(key, value) {
        this.#items = this.#items.filter(pair => pair[0] !== String(key) || (value !== undefined && pair[1] !== String(value)));
        this.#notify();
    }
    set(key, value) {
        key = String(key); value = String(value);
        const at = this.#items.findIndex(pair => pair[0] === key);
        if (at < 0) this.#items.push([key,value]);
        else { this.#items[at][1] = value; this.#items = this.#items.filter((pair,i) => pair[0] !== key || i === at); }
        this.#notify();
    }
    sort() { this.#items.sort((a,b) => a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0); this.#notify(); }
    *entries() { for (let i=0; i<this.#items.length; i++) yield [...this.#items[i]]; }
    *keys() { for (const pair of this.#items) yield pair[0]; }
    *values() { for (const pair of this.#items) yield pair[1]; }
    [Symbol.iterator]() { return this.entries(); }
    forEach(callback, thisArg) { for (const [key,value] of this) callback.call(thisArg,value,key,this); }
    toString() { return __query(JSON.stringify(this.#items)); }
};

globalThis.URL = class URL {
    #fields;
    #params;
    constructor(value, base) {
        const fields = JSON.parse(__url(String(value), base === undefined ? null : String(base)));
        if (fields.error) throw new TypeError(fields.error);
        this.#fields = fields;
        this.#params = new URLSearchParams(fields.search);
        this.#params._connect(query => this.#set("search", query ? `?${query}` : ""));
    }
    #set(key, value) {
        const fields = JSON.parse(__urlSet(this.href,key,String(value)));
        if (fields.error) throw new TypeError(fields.error);
        this.#fields = fields;
        this.#params._replace(fields.search);
    }
    get href() { return this.#fields.href; } set href(v) { this.#set("href",v); }
    get origin() { return this.#fields.origin; }
    get protocol() { return this.#fields.protocol; } set protocol(v) { this.#set("protocol",v); }
    get username() { return this.#fields.username; } set username(v) { this.#set("username",v); }
    get password() { return this.#fields.password; } set password(v) { this.#set("password",v); }
    get host() { return this.#fields.host; } set host(v) { this.#set("host",v); }
    get hostname() { return this.#fields.hostname; } set hostname(v) { this.#set("hostname",v); }
    get port() { return this.#fields.port; } set port(v) { this.#set("port",v); }
    get pathname() { return this.#fields.pathname; } set pathname(v) { this.#set("pathname",v); }
    get search() { return this.#fields.search; } set search(v) { this.#set("search",v); }
    get hash() { return this.#fields.hash; } set hash(v) { this.#set("hash",v); }
    get searchParams() { return this.#params; }
    toString() { return this.href; }
    toJSON() { return this.href; }
    static canParse(value,base) { try { new URL(value,base); return true; } catch { return false; } }
    static parse(value,base) { try { return new URL(value,base); } catch { return null; } }
};

globalThis.TextEncoder = class TextEncoder {
    get encoding() { return "utf-8"; }
    encode(input = "") {
        const out = [];
        for (const char of String(input)) out.push(...this.#bytes(char.codePointAt(0)));
        return new Uint8Array(out);
    }
    #bytes(code) {
        if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
        if (code < 0x80) return [code];
        if (code < 0x800) return [0xc0|(code>>6),0x80|(code&63)];
        if (code < 0x10000) return [0xe0|(code>>12),0x80|((code>>6)&63),0x80|(code&63)];
        return [0xf0|(code>>18),0x80|((code>>12)&63),0x80|((code>>6)&63),0x80|(code&63)];
    }
    encodeInto(input,destination) {
        if (!(destination instanceof Uint8Array)) throw new TypeError("Expected Uint8Array");
        let read=0, written=0;
        for (const char of String(input)) {
            const bytes=this.#bytes(char.codePointAt(0));
            if (written+bytes.length > destination.length) break;
            destination.set(bytes,written); written+=bytes.length; read+=char.length;
        }
        return {read,written};
    }
};

globalThis.TextDecoder = class TextDecoder {
    #pending=[];
    #bomSeen=false;
    #fatal;
    #ignoreBOM;
    constructor(label="utf-8",options={}) {
        if (!["utf-8","utf8","unicode-1-1-utf-8"].includes(String(label).trim().toLowerCase())) throw new RangeError("Unsupported text encoding");
        this.#fatal=Boolean(options.fatal); this.#ignoreBOM=Boolean(options.ignoreBOM);
    }
    get encoding() { return "utf-8"; }
    get fatal() { return this.#fatal; }
    get ignoreBOM() { return this.#ignoreBOM; }
    decode(input=new Uint8Array(),options={}) {
        let bytes;
        if (input instanceof ArrayBuffer) bytes=new Uint8Array(input);
        else if (ArrayBuffer.isView(input)) bytes=new Uint8Array(input.buffer,input.byteOffset,input.byteLength);
        else throw new TypeError("Expected an ArrayBuffer or view");
        const result=JSON.parse(__decodeUtf8(JSON.stringify([...this.#pending,...bytes]),this.fatal,Boolean(options.stream)));
        if (result.error) { this.#pending=[]; this.#bomSeen=false; throw new TypeError(result.error); }
        this.#pending=result.remaining;
        let text=result.text;
        if (!this.#bomSeen && text.length) { this.#bomSeen=true; if (!this.ignoreBOM && text[0] === "\uFEFF") text=text.slice(1); }
        if (!options.stream) { this.#pending=[]; this.#bomSeen=false; }
        return text;
    }
};
