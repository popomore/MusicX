export function probe() {
  const url = new URL("../search?q=中文", "https://example.com/base/path");
  url.searchParams.set("x", 2);
  const relative = url.href;
  url.hash = "part";
  const fields = [url.origin, url.pathname, url.search, url.hash, url.searchParams.get("q"), url.searchParams.has("x")];
  const paramsRef = url.searchParams;
  url.pathname = "/next";
  url.search = "a=2&a=3";
  url.hash = "";
  const live = [paramsRef === url.searchParams, url.href, paramsRef.getAll("a")];
  paramsRef.set("a", 4);
  live.push(url.href);
  url.href = "https://other.example/a?x=5";
  live.push(url.toJSON());
  const q = new URLSearchParams("?space=space+a&dup=1&dup=2");
  const params = [q.get("space"), q.getAll("dup"), q.has("dup", "2")];
  q.delete("dup", "1"); q.set("dup", "3"); q.sort();
  params.push(q.toString(), q.size);
  const pairs = new URLSearchParams([["x",1],["x",2]]);
  const iterable = [pairs.toString(), [...pairs.keys()], [...pairs.values()], [...pairs]];
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const utf8 = [Array.from(encoder.encode("中文🎵")), decoder.decode(encoder.encode("中文🎵")), Array.from(encoder.encode("\uD800")), decoder.decode(new Uint8Array([0xe2,0x28,0xa1])), decoder.decode(new Uint8Array([0xef,0xbb,0xbf,65]))];
  const streaming = [decoder.decode(new Uint8Array([0xe4,0xb8]), {stream:true}), decoder.decode(new Uint8Array([0xad]), {stream:true}), decoder.decode(new Uint8Array([0xe6]), {stream:true}), decoder.decode(new Uint8Array([0x96,0x87]))];
  const short = new Uint8Array(3), enough = new Uint8Array(4);
  const encode_into = [encoder.encodeInto("A🎵",short), Array.from(short), encoder.encodeInto("🎵",enough), Array.from(enough)];
  const invalid = [];
  try { new URL("/relative"); invalid.push(false); } catch { invalid.push(true); }
  try { new TextDecoder("not-an-encoding"); invalid.push(false); } catch { invalid.push(true); }
  try { new TextDecoder("utf-8",{fatal:true}).decode(new Uint8Array([0xff])); invalid.push(false); } catch { invalid.push(true); }
  const base64=[btoa('\u0000\u00ff\u0080A'),Array.from(atob('AP+AQQ=='),c=>c.charCodeAt(0))];
  try {atob('a');invalid.push(false);}catch{invalid.push(true);}
  return {relative, fields, live, params, iterable, utf8, streaming, encode_into, invalid,base64};
}

export function storage_limits() {
  host.storage.set("catalog", "a".repeat(96000));
  let rejected=false;
  try { host.storage.set("catalog","b".repeat(4*1024*1024)); } catch { rejected=true; }
  return {rejected,retained:host.storage.get("catalog").length};
}

export function stored() { return host.storage.get("catalog").length; }

export async function aborted_requests(){
  const poll=globalThis.__fetchPoll;globalThis.__fetchPoll=()=>null;
  const errors=[];try{for(let i=0;i<20;i++){
    const controller=new AbortController();const pending=fetch('https://example.com/not-permitted',{signal:controller.signal});controller.abort();
    try{await pending;}catch(error){errors.push(error.message);}
  }}finally{globalThis.__fetchPoll=poll;}
  return errors;
}

export async function form_fetch() {
  let request;
  globalThis.__fetchStart=undefined;
  globalThis.__fetch=raw=>{
    request=JSON.parse(raw);
    return JSON.stringify({status:200,body:"{}",headers:[["set-cookie","a=1; Expires=Thu, 01 Jan 2037 00:00:00 GMT"],["set-cookie","b=2"]]});
  };
  const response=await fetch("https://example.com/form",{method:"POST",headers:[["X-Test","yes"]],body:new URLSearchParams({q:"a b",lang:"中文"})});
  return {request,cookies:response.headers.getSetCookie()};
}
