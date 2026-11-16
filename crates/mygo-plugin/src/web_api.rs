//! WHATWG URL parsing and UTF-8 decoding shared by the plugin Web API bindings.
use rquickjs::{Ctx, Function};
use serde_json::{Value, json};
use url::Url;

fn fields(url: &Url) -> Value {
    json!({
        "href":url.as_str(), "origin":url.origin().ascii_serialization(),
        "protocol":format!("{}:",url.scheme()), "username":url.username(),
        "password":url.password().unwrap_or_default(),
        "hostname":url.host_str().unwrap_or_default(),
        "host":&url[url::Position::BeforeHost..url::Position::AfterPort],
        "port":url.port().map(|v|v.to_string()).unwrap_or_default(),
        "pathname":url.path(),
        "search":url.query().filter(|v|!v.is_empty()).map(|v|format!("?{v}")).unwrap_or_default(),
        "hash":url.fragment().filter(|v|!v.is_empty()).map(|v|format!("#{v}")).unwrap_or_default()
    })
}

fn parse(value: &str, base: Option<&str>) -> Value {
    let url = match base {
        None => Url::parse(value),
        Some(base) => Url::parse(base).and_then(|base| base.join(value)),
    };
    match url {
        Ok(url) => fields(&url),
        Err(_) => json!({"error":"Invalid URL"}),
    }
}

fn mutate(href: &str, component: &str, value: &str) -> Value {
    let Ok(mut url) = Url::parse(href) else {
        return json!({"error":"Invalid URL"});
    };
    match component {
        "href" => return parse(value, None),
        "protocol" => {
            let _ = url.set_scheme(value.trim_end_matches(':'));
        }
        "username" => {
            let _ = url.set_username(value);
        }
        "password" => {
            let _ = url.set_password(Some(value));
        }
        "hostname" => {
            let _ = url.set_host(Some(value));
        }
        "host" => {
            if let Ok(host) = Url::parse(&format!("{}://{value}/", url.scheme()))
                && host.username().is_empty()
                && host.password().is_none()
            {
                let _ = url.set_host(host.host_str());
                let _ = url.set_port(host.port());
            }
        }
        "port" => {
            if value.is_empty() {
                let _ = url.set_port(None);
            } else if let Ok(port) = value.parse::<u16>() {
                let _ = url.set_port(Some(port));
            }
        }
        "pathname" => url.set_path(value),
        "search" => url.set_query(if value.is_empty() {
            None
        } else {
            Some(value.strip_prefix('?').unwrap_or(value))
        }),
        "hash" => url.set_fragment(if value.is_empty() {
            None
        } else {
            Some(value.strip_prefix('#').unwrap_or(value))
        }),
        _ => return json!({"error":"Unknown URL component"}),
    }
    fields(&url)
}

fn decode(bytes: &[u8], fatal: bool, stream: bool) -> Value {
    let mut text = String::new();
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(valid) => {
                text.push_str(valid);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                text.push_str(std::str::from_utf8(&bytes[offset..offset + valid]).unwrap());
                offset += valid;
                if error.error_len().is_none() && stream {
                    return json!({"text":text,"remaining":&bytes[offset..]});
                }
                if fatal {
                    return json!({"error":"The encoded data was not valid UTF-8"});
                }
                text.push('\u{fffd}');
                offset += error.error_len().unwrap_or(bytes.len() - offset);
            }
        }
    }
    json!({"text":text,"remaining":[]})
}

pub(crate) fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let global = ctx.globals();
    global.set(
        "__url",
        Function::new(ctx.clone(), |value: String, base: Option<String>| {
            parse(&value, base.as_deref()).to_string()
        })?,
    )?;
    global.set(
        "__urlSet",
        Function::new(ctx.clone(), |href: String, key: String, value: String| {
            mutate(&href, &key, &value).to_string()
        })?,
    )?;
    global.set(
        "__queryParse",
        Function::new(ctx.clone(), |text: String| {
            serde_json::to_string(
                &url::form_urlencoded::parse(text.as_bytes())
                    .into_owned()
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        })?,
    )?;
    global.set(
        "__decodeUtf8",
        Function::new(ctx.clone(), |text: String, fatal: bool, stream: bool| {
            match serde_json::from_str::<Vec<u8>>(&text) {
                Ok(bytes) => decode(&bytes, fatal, stream),
                Err(_) => json!({"error":"Expected bytes"}),
            }
            .to_string()
        })?,
    )?;
    ctx.eval::<(), _>(include_str!("web_api.js"))
}
