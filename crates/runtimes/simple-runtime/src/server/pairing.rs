use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

const WINDOW: Duration = Duration::from_secs(30);

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
#[derive(Clone)]
pub struct FakeClock {
    base: Instant,
    offset: Arc<Mutex<Duration>>,
}

#[cfg(test)]
impl FakeClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: Arc::new(Mutex::new(Duration::ZERO)),
        }
    }

    pub fn advance(&self, by: Duration) {
        *self.offset.lock() += by;
    }
}

#[cfg(test)]
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.base + *self.offset.lock()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    NoWindow,
    Expired,
    WrongCode,
}

impl PairError {
    pub fn message(&self) -> &'static str {
        match self {
            PairError::NoWindow => "no pairing window",
            PairError::Expired => "pairing window expired",
            PairError::WrongCode => "wrong pairing code",
        }
    }
}

struct Window {
    code: String,
    expires_at: Instant,
}

pub struct TokenStore {
    path: PathBuf,
    tokens: Mutex<HashSet<String>>,
}

impl TokenStore {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join("tokens.json");
        let tokens = if path.exists() {
            let data = fs::read_to_string(&path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashSet::new()
        };
        Ok(Self {
            path,
            tokens: Mutex::new(tokens),
        })
    }

    pub fn contains(&self, token: &str) -> bool {
        self.tokens.lock().contains(token)
    }

    pub fn insert(&self, token: String) -> std::io::Result<()> {
        let mut g = self.tokens.lock();
        g.insert(token);
        let data = serde_json::to_vec_pretty(&*g).unwrap_or_else(|_| b"[]".to_vec());
        fs::write(&self.path, data)
    }
}

pub struct Pairing<C: Clock> {
    clock: C,
    window: Mutex<Option<Window>>,
    tokens: Arc<TokenStore>,
}

impl Pairing<SystemClock> {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        Ok(Self::with_clock(SystemClock, Arc::new(TokenStore::open(dir)?)))
    }
}

impl<C: Clock> Pairing<C> {
    pub fn with_clock(clock: C, tokens: Arc<TokenStore>) -> Self {
        Self {
            clock,
            window: Mutex::new(None),
            tokens,
        }
    }

    pub fn tokens(&self) -> Arc<TokenStore> {
        self.tokens.clone()
    }

    pub fn request(&self) -> String {
        let n: u32 = rand::random::<u32>() % 1_000_000;
        let code = format!("{n:06}");
        let expires_at = self.clock.now() + WINDOW;
        *self.window.lock() = Some(Window {
            code: code.clone(),
            expires_at,
        });
        // Print both streams so the operator sees the code on whichever one they watch.
        println!("PAIRING {code}");
        eprintln!("PAIRING {code}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let _ = std::io::Write::flush(&mut std::io::stderr());
        code
    }

    pub fn confirm(&self, code: &str) -> Result<String, PairError> {
        let mut g = self.window.lock();
        let window = g.take().ok_or(PairError::NoWindow)?;
        if self.clock.now() >= window.expires_at {
            return Err(PairError::Expired);
        }
        if window.code != code {
            *g = Some(window);
            return Err(PairError::WrongCode);
        }
        drop(g);
        let bytes: [u8; 32] = rand::random();
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        self.tokens
            .insert(token.clone())
            .map_err(|_| PairError::NoWindow)?;
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_tokens() -> TokenStore {
        let dir = std::env::temp_dir().join(format!("it-pair-{}", UuidLike::new()));
        TokenStore::open(&dir).unwrap()
    }

    struct UuidLike;
    impl UuidLike {
        fn new() -> String {
            uuid::Uuid::new_v4().to_string()
        }
    }

    #[test]
    fn wrong_code_is_rejected_and_window_stays() {
        let p = Pairing::with_clock(SystemClock, Arc::new(tmp_tokens()));
        let code = p.request();
        assert_eq!(p.confirm("000000"), Err(PairError::WrongCode));
        let token = p.confirm(&code).unwrap();
        assert_eq!(token.len(), 64);
        assert!(p.tokens().contains(&token));
    }

    #[test]
    fn window_expires_after_30s_with_injected_clock() {
        let clock = FakeClock::new();
        let p = Pairing::with_clock(clock.clone(), Arc::new(tmp_tokens()));
        let code = p.request();
        clock.advance(Duration::from_secs(31));
        assert_eq!(p.confirm(&code), Err(PairError::Expired));
    }

    #[test]
    fn confirm_without_request_fails() {
        let p = Pairing::with_clock(SystemClock, Arc::new(tmp_tokens()));
        assert_eq!(p.confirm("123456"), Err(PairError::NoWindow));
    }
}
