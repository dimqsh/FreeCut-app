//! Shared helpers for the offline integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)] // test code

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use freecut_cloud::CancelToken;
use freecut_cloud::catalogue::ModelId;
use freecut_cloud::error::CloudError;
use freecut_cloud::fake::{FakeService, ToolReply};
use freecut_cloud::mcp::{TokenPurpose, TokenSource};
use freecut_cloud::provider::higgsfield::{ENDPOINT, HiggsfieldProvider};
use freecut_cloud::provider::{
    Aspect, ImageRequest, Operation, ParamValues, VideoMode, VideoRequest, WholeSeconds,
};
use freecut_cloud::secret::Secret;
use freecut_cloud::transport::Waiter;
use serde_json::{Value, json};

pub fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/higgsfield/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

pub fn fixture_json(name: &str) -> Value {
    serde_json::from_str(&fixture(name)).unwrap()
}

pub fn golden(name: &str) -> Value {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap()
}

/// Records waits instead of sleeping.
#[derive(Debug, Default)]
pub struct NoWait {
    pub waits: Mutex<Vec<Duration>>,
}

impl Waiter for NoWait {
    fn wait(&self, duration: Duration, cancel: &CancelToken) -> bool {
        self.waits.lock().unwrap().push(duration);
        !cancel.is_cancelled()
    }

    fn jitter(&self) -> u32 {
        0
    }
}

/// A token source that swaps to `next` on renewal, once.
#[derive(Debug)]
pub struct SwapToken {
    current: Mutex<String>,
    next: Mutex<Option<String>>,
    pub renewals: AtomicUsize,
}

impl SwapToken {
    pub fn new(current: &str, next: Option<&str>) -> Arc<SwapToken> {
        Arc::new(SwapToken {
            current: Mutex::new(current.into()),
            next: Mutex::new(next.map(str::to_owned)),
            renewals: AtomicUsize::new(0),
        })
    }

    pub fn renewals(&self) -> usize {
        self.renewals.load(Ordering::SeqCst)
    }
}

impl TokenSource for SwapToken {
    fn bearer(&self, _purpose: TokenPurpose, _cancel: &CancelToken) -> Result<Secret, CloudError> {
        Ok(Secret::new(self.current.lock().unwrap().clone()))
    }

    fn renew(&self, _cancel: &CancelToken) -> Result<(), CloudError> {
        self.renewals.fetch_add(1, Ordering::SeqCst);
        match self.next.lock().unwrap().take() {
            Some(next) => {
                *self.current.lock().unwrap() = next;
                Ok(())
            }
            None => Err(CloudError::SignInExpired),
        }
    }
}

/// The recorded tool schemas, plus a denied tool to check filtering.
pub fn tools() -> Vec<Value> {
    let mut tools = fixture_json("tools-list-schemas.json")["tools"]
        .as_array()
        .unwrap()
        .clone();
    tools.push(
        json!({"name": "execute_preset", "description": "", "inputSchema": {"type": "object"}}),
    );
    tools
}

/// A fake Higgsfield that accepts token `t1`, lists the recorded tools,
/// serves the recorded catalogue, and answers cost checks with the recorded
/// figures. A paid call (no `get_cost`) fails the test.
pub fn fake_higgsfield() -> FakeService {
    let fake = FakeService::new(ENDPOINT);
    fake.accept_token("t1");
    fake.set_tools(tools(), 2);
    fake.on_tool("models_explore", |args| {
        let kind = args["type"].as_str().unwrap_or_default();
        ToolReply::Json(fixture_json(&format!("models_explore-list-{kind}.json")))
    });
    for (tool, answer) in [
        ("generate_image", "get_cost-generate_image.json"),
        ("generate_video", "get_cost-generate_video.json"),
    ] {
        fake.on_tool(tool, move |args| {
            assert_eq!(
                args["params"]["get_cost"],
                json!(true),
                "a paid call reached the fake: {args}"
            );
            ToolReply::Json(fixture_json(answer))
        });
    }
    fake
}

pub fn provider(
    fake: &FakeService,
    tokens: Arc<dyn TokenSource>,
) -> (HiggsfieldProvider, Arc<NoWait>) {
    let waiter = Arc::new(NoWait::default());
    let provider = HiggsfieldProvider::with_endpoint(
        ENDPOINT,
        Arc::new(fake.clone()),
        tokens,
        waiter.clone(),
        "0.1.0",
    );
    (provider, waiter)
}

pub fn image_op() -> Operation {
    Operation::Image(ImageRequest {
        model: ModelId::new("gpt_image_2_5"),
        prompt: "a red apple on a white table".into(),
        refs: vec![],
        aspect: Aspect::parse("1:1"),
        variants: 1,
        params: ParamValues::new(),
    })
}

pub fn video_op() -> Operation {
    Operation::Video(VideoRequest {
        model: ModelId::new("seedance_2_5"),
        mode: VideoMode::TextToVideo,
        prompt: "waves rolling onto a beach at sunset".into(),
        refs: vec![],
        aspect: Aspect::parse("16:9"),
        duration: Some(WholeSeconds(5)),
        resolution: Some("720p".into()),
        sound: None,
        variants: 1,
        params: ParamValues::new(),
    })
}
