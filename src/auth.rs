use crate::{Result, UklientError, CLIENT};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use theseus::prelude::Credentials;
use tokio::time::interval;

const CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBOX_USER_AUTH_URL: &str =
    "https://user.auth.xboxlive.com/user/authenticate";
const XBOX_AUTHORIZATION_URL: &str =
    "https://xsts.auth.xboxlive.com/xsts/authorize";
const YGGDRASIL_URL: &str = "https://api.minecraftservices.com/launcher/login";
const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

const CLIENT_ID: &str = "89f4991d-b660-41c0-9ee4-affe27d47bce";
const DEFAULT_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Token {
    expires_in: i64,
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileInfo {
    id: uuid::Uuid,
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct XboxTokenInfo {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct YggdrasilToken {
    access_token: String,
    expires_in: i64,
    username: uuid::Uuid,
}

pub async fn get_device_code(scopes: Vec<&str>) -> Result<DeviceCode> {
    let scopes = scopes.join(" ");
    let response = CLIENT
        .get(CODE_URL)
        .query(&[("client_id", CLIENT_ID), ("scope", scopes.as_str())])
        .send()
        .await?;
    let code: DeviceCode = response.json().await?;

    Ok(code)
}

pub async fn get_credentials(device_code: String) -> Result<Credentials> {
    let body = [
        ("client_id", CLIENT_ID),
        ("code", device_code.as_str()),
        ("grant_type", DEFAULT_GRANT),
    ];
    let mut interval = interval(std::time::Duration::from_secs(5));

    let token = loop {
        let response = CLIENT.post(TOKEN_URL).form(&body).send().await?;
        let text = response.text().await?;

        match serde_json::from_str::<Token>(&text) {
            Ok(t) => {
                break t;
            }
            Err(_) => {
                let json: Map<String, Value> = serde_json::from_str(&text)?;

                if json
                    .get("error")
                    .and_then(|v| v.as_str())
                    .filter(|&s| s == "authorization_pending")
                    .is_some()
                {
                    interval.tick().await;
                } else {
                    return Err(UklientError::LoginError(text));
                }
            }
        }
    };

    let expiration = Utc::now() + Duration::seconds(token.expires_in);
    let info = fetch_info(&token.access_token).await?;

    Ok(Credentials {
        username: info.name,
        id: info.id,
        refresh_token: token.refresh_token,
        access_token: token.access_token,
        expires: expiration,
        _ctor_scope: std::marker::PhantomData,
    })
}

async fn fetch_info(token: &str) -> Result<ProfileInfo> {
    let xbox_token = get_xbox_token(token).await?;
    let mojang_token = get_mojang_services_token(&xbox_token.token).await?;
    let yggdrasil_token = get_yggdrasil_token(&mojang_token).await?;

    Ok(CLIENT
        .get(PROFILE_URL)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", yggdrasil_token.access_token),
        )
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn get_xbox_token(token: &str) -> Result<XboxTokenInfo> {
    let body = format!(
        r#"{{
        "Properties": {{
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": "d={token}"
        }},
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    }}"#
    );

    Ok(CLIENT
        .post(XBOX_USER_AUTH_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn get_mojang_services_token(token: &str) -> Result<XboxTokenInfo> {
    let body = format!(
        r#"{{
        "Properties": {{
            "SandboxId": "RETAIL",
            "UserTokens": ["{token}"]
        }},
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    }}"#
    );

    Ok(CLIENT
        .post(XBOX_AUTHORIZATION_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn get_yggdrasil_token(token: &XboxTokenInfo) -> Result<YggdrasilToken> {
    let body = format!(
        r#"{{
        "xtoken": "XBL3.0 x={};{}",
        "platform": "PC_LAUNCHER"
    }}"#,
        token.display_claims["xui"][0]["uhs"].as_str().unwrap_or(""),
        token.token
    );

    Ok(CLIENT
        .post(YGGDRASIL_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

// flow:
// 1. get device code
// 2. get xbox user token
// 3. get mojang services token
// 4. get yggdrasil token
// 5. fetch profile info
