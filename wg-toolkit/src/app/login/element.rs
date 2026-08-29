//! Definition of elements related to login application.
//! 
//! When a client send a login request to the login app, it might be encrypted with RSA, 
//! the server then decide which response to return depending on the input, it might 
//! send a challenge that is required. When the login succeed, the server sends a login 
//! key that is used by the client when first connecting to the base app.
//! 
//! This app also provides a way to ping test the server.

use std::io::{self, Read, Write};
use std::time::Duration;

use rsa::{RsaPrivateKey, RsaPublicKey};
use blowfish::Blowfish;

use crate::net::filter::{RsaWriter, RsaReader, BlowfishWriter, BlowfishReader};
use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, SimpleElement};
use crate::net::codec::{Codec, SimpleCodec, WgSocketAddrV4};
use crate::util::io::*;


/// Internal module containing all raw elements numerical ids.
///
/// All ids below were confirmed empirically by attaching to a live game process
/// (v2.3.1.3, 2026-08-24) and reading the actual registration order out of
/// `LoginInterface`'s `Mercury::InterfaceMinder` in memory.
pub mod id {
    pub const LOGIN_REQUEST: u8         = 0x00;
    pub const PROBE: u8                 = 0x01;
    pub const PING: u8                  = 0x02;
    pub const CHALLENGE_RESPONSE: u8    = 0x03;
    pub const MTU_PROBE: u8             = 0x04;
}


crate::__struct_simple_codec! {
    /// A ping sent from the client to the login app or replied from the
    /// login app to the client.
    #[derive(Debug, Clone, Copy)]
    pub struct Ping {
        /// The number of the ping, the same number must be sent back to
        /// the client when login app receives it.
        pub num: u8,
    }
}

impl SimpleElement for Ping {
    const ID: u8 = id::PING;
    const LEN: ElementLength = ElementLength::Fixed(1);
}


/// Sent from the client to the login app to request basic server info (host name,
/// owner name, users count) without going through the full login process. Not yet
/// given a proper structured codec.
pub type Probe = DebugElementFixed<{ id::PROBE }, 0>;

/// Presumably used to probe path MTU before/during login. Not yet given a proper
/// structured codec.
pub type MtuProbe = DebugElementVariable16<{ id::MTU_PROBE }>;


/// A login request to be sent with [`LoginCodec`], sent from client to 
/// server when it wants to log into and gain access to a base app.
#[derive(Debug, Default, Clone)]
pub struct LoginRequest {
    /// The protocol used, currently undocumented.
    pub protocol: u32,
    /// The username used to login.
    pub username: String,
    /// The password used to login, only used when using password to login.
    pub password: String,
    /// The blowfish key to initialize the blowfish cipher, its size can be between 
    /// 4 and 56, both included, the full key is 56 bytes long.
    pub blowfish_key: Vec<u8>,
    pub context: String,
    pub digest: Option<[u8; 16]>,
    pub nonce: u32,
}

/// Implementation without encryption!
impl Codec<()> for LoginRequest {

    fn write(&self, write: &mut dyn Write, _config: &()) -> io::Result<()> {
        write.write_u32(self.protocol)?;
        write.write_bool(false)?;
        write_login_request(write, self)
    }

    fn read(read: &mut dyn Read, _config: &()) -> io::Result<Self> {
        let protocol = read.read_u32()?;
        if read.read_bool()? {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "missing private encryption key to decode the login request"));
        }
        read_login_request(read, protocol)
    }

}

/// Implementation with only client-side encryption.
impl Codec<RsaPublicKey> for LoginRequest {

    fn write(&self, write: &mut dyn Write, config: &RsaPublicKey) -> io::Result<()> {
        write.write_u32(self.protocol)?;
        write.write_bool(true)?;
        write_login_request(&mut RsaWriter::new(write, config), self)
    }

    fn read(read: &mut dyn Read, _config: &RsaPublicKey) -> io::Result<Self> {
        let protocol = read.read_u32()?;
        if read.read_bool()? {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "only private encryption key can decode login request"));
        }
        read_login_request(read, protocol)
    }

}

/// Implementation with both server-side decryption and client-side encryption (derived
/// from private key).
impl Codec<RsaPrivateKey> for LoginRequest {

    fn write(&self, write: &mut dyn Write, config: &RsaPrivateKey) -> io::Result<()> {
        write.write_u32(self.protocol)?;
        write.write_bool(true)?;
        write_login_request(&mut RsaWriter::new(write, &config.to_public_key()), self)
    }

    fn read(read: &mut dyn Read, config: &RsaPrivateKey) -> io::Result<Self> {
        let protocol = read.read_u32()?;
        if read.read_bool()? {
            read_login_request(&mut RsaReader::new(read, config), protocol)
        } else {
            read_login_request(read, protocol)
        }
    }

}

fn write_login_request(write: &mut dyn Write, input: &LoginRequest) -> io::Result<()> {
    write.write_u8(if input.digest.is_some() { 0x01 } else { 0x00 })?;
    write.write_string_variable(&input.username)?;
    write.write_string_variable(&input.password)?;
    write.write_blob_variable(&input.blowfish_key)?;
    write.write_string_variable(&input.context)?;
    if let Some(digest) = input.digest {
        write.write_all(&digest)?;
    }
    write.write_u32(input.nonce)
}

fn read_login_request(input: &mut dyn Read, protocol: u32) -> io::Result<LoginRequest> {
    let flags = input.read_u8()?;
    Ok(LoginRequest {
        protocol,
        username: input.read_string_variable()?,
        password: input.read_string_variable()?,
        blowfish_key: input.read_blob_variable()?,
        context: input.read_string_variable()?,
        digest: if flags & 0x01 != 0 {
            let mut digest = [0; 16];
            input.read_exact(&mut digest)?;
            Some(digest)
        } else {
            Option::None
        },
        nonce: input.read_u32()?
    })
}

impl<C> SimpleElement<C> for LoginRequest
where LoginRequest: Codec<C> {
    const ID: u8 = id::LOGIN_REQUEST;
    const LEN: ElementLength = ElementLength::Variable16;
}


/// Describe all kinds of responses returned from server to client when
/// the client attempt to login. This includes challenge or error codes.
#[derive(Debug, Clone)]
pub enum LoginResponse {
    /// The login is successful.
    Success(LoginSuccess),
    /// A challenge must be completed in order to have a response.
    Challenge(LoginChallenge),
    /// An error happened server-side and the login process cannot succeed. This data
    /// is expected to be a JSON string on modern version of the game.
    Error(LoginError, String),
    /// Unknown response code.
    Unknown(u8),
}

/// Describe a login success response. It provides the client with the
/// address of the base app to connect, session key and an optional
/// server message.
#[derive(Debug, Clone)]
pub struct LoginSuccess {
    /// The socket address of the base app server to connect after successful
    /// login.
    pub addr: WgSocketAddrV4,
    /// Session key, it's used to authenticate to the base app.
    pub login_key: u32,
    /// Server message for successful login.
    pub server_message: String,
}

/// Describe an issued challenge as a response to a login request.
#[derive(Debug, Clone)]
pub enum LoginChallenge {
    /// Cuckoo cycle challenge.
    CuckooCycle {
        /// This prefix string to the key used to initialize the Cuckoo Cycle context,
        /// it's given to a SHA-256 before being used, so it can be any size.
        key_prefix: Vec<u8>,
        max_nonce: u32,
    },
}

/// Describe a login error as a response to a login request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LoginError {
    MalformedRequest = 64,
    BadProtocolVersion = 65,
    // ChallengeIssued = 66, handled by a specific variant of LoginResponse.
    InvalidUser = 67,
    InvalidPassword = 68,
    AlreadyLoggedIn = 69,
    BadDigest = 70,
    DatabaseGeneralFailure = 71,
    DatabaseNotReady = 72,
    IllegalCharacters = 73,
    ServerNotReady = 74,
    UpdaterNotReady = 75, // No longer used
    NoBaseApp = 76,
    BaseAppOverload = 77,
    CellAppOverload = 78,
    BaseAppTimeout = 79,
    BaseAppManagerTimeout = 80,
    DatabaseAppOverload = 81,
    LoginNotAllowed = 82,
    /// This error is handled in a specific way, the client will put the client in a
    /// waiting mode where it will automatically retry login.
    RateLimited = 83,
    /// This error expect the message to be a JSON containing a single key 'bans' that
    /// is itself a string representation of a JSON element containing 'expiryTime' 
    /// and 'reason'.
    Banned = 84,
    ChallengeError = 85,
}

/// Text identifier of the cuckoo cycle challenge type.
const CHALLENGE_CUCKOO_CYCLE: &'static str = "cuckoo_cycle";

impl LoginResponse {

    fn write_inner(&self, write: &mut dyn Write, bf: Option<&Blowfish>) -> io::Result<()> {

        match self {
            Self::Success(success) => {
                
                write.write_u8(1)?; // Logged-on
                
                if let Some(bf) = bf {
                    write_login_success(&mut BlowfishWriter::new(write, bf), success)?;
                } else {
                    write_login_success(write, success)?;
                }
    
            }
            Self::Challenge(challenge) => {
    
                write.write_u8(66)?;
                
                match challenge {
                    LoginChallenge::CuckooCycle { key_prefix: prefix, max_nonce } => {
                        write.write_string_variable(CHALLENGE_CUCKOO_CYCLE)?;
                        write.write_blob_variable(&prefix)?;
                        write.write_u64(*max_nonce as u64)?;
                    }
                }
                
            }
            Self::Error(err, message) => {
                write.write_u8(*err as _)?;
                write.write_string_variable(&message)?;
            }
            Self::Unknown(code) => write.write_u8(*code)?
        }
    
        Ok(())

    }

    fn read_inner(read: &mut dyn Read, bf: Option<&Blowfish>) -> io::Result<Self> {

        let error = match read.read_u8()? {
            1 => {
                
                let success = 
                if let Some(bf) = bf {
                    read_login_success(&mut BlowfishReader::new(read, bf))?
                } else {
                    read_login_success(read)?
                };

                return Ok(LoginResponse::Success(success));

            }
            66 => {
                
                let challenge_name = read.read_string_variable()?;
                let challenge = match &challenge_name[..] {
                    CHALLENGE_CUCKOO_CYCLE => {
                        let prefix = read.read_blob_variable()?;
                        let max_nonce = read.read_u64()? as u32;
                        LoginChallenge::CuckooCycle { 
                            key_prefix: prefix, 
                            max_nonce,
                        }
                    }
                    name => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid challenge name: {name}")))
                };

                return Ok(LoginResponse::Challenge(challenge));

            }
            64 => LoginError::MalformedRequest,
            65 => LoginError::BadProtocolVersion,
            67 => LoginError::InvalidUser,
            68 => LoginError::InvalidPassword,
            // TODO: Implement other variants
            code => return Ok(LoginResponse::Unknown(code))
        };

        let message = match read.read_string_variable() {
            Ok(msg) => msg,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => String::new(),
            Err(e) => return Err(e),
        };

        Ok(LoginResponse::Error(error, message))

    }

}

/// Login response without decryption capability for login success.
impl Codec<()> for LoginResponse {

    fn write(&self, write: &mut dyn Write, _config: &()) -> io::Result<()> {
        Self::write_inner(self, write, None)
    }

    fn read(read: &mut dyn Read, _config: &()) -> io::Result<Self> {
        Self::read_inner(read, None)
    }

}

impl Codec<Blowfish> for LoginResponse {

    fn write(&self, write: &mut dyn Write, config: &Blowfish) -> io::Result<()> {
        Self::write_inner(self, write, Some(config))
    }

    fn read(read: &mut dyn Read, config: &Blowfish) -> io::Result<Self> {
        Self::read_inner(read, Some(config))
    }

}

/// Internal function for encoding login success. It is extracted here
/// in order to be usable with optional encryption.
fn write_login_success(write: &mut dyn Write, success: &LoginSuccess) -> io::Result<()> {
    SimpleCodec::write(&success.addr, write)?;
    write.write_u32(success.login_key)?;
    if !success.server_message.is_empty() {
        write.write_string_variable(&success.server_message)?;
    }
    Ok(())
}

/// Internal function for decoding login success. It is extracted here
/// in order to be usable with optional encryption.
fn read_login_success(read: &mut dyn Read) -> io::Result<LoginSuccess> {
    Ok(LoginSuccess {
        addr: SimpleCodec::read(read)?,
        login_key: read.read_u32()?,
        server_message: match read.read_string_variable() {
            Ok(msg) => msg,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => String::new(),
            Err(e) => return Err(e),
        },
    })
}


/// Describe a generic challenge response of a given generic type. This is an element
/// that's not expecting any reply, because the client sends a new login request just
/// after.
#[derive(Debug, Clone)]
pub struct ChallengeResponse<D> {
    /// Resolve duration of the challenge.
    pub duration: Duration,
    /// Inner data of the challenge response.
    pub data: D,
}

impl<D: Codec<C>, C> Codec<C> for ChallengeResponse<D> {

    fn write(&self, write: &mut dyn Write, config: &C) -> io::Result<()> {
        write.write_f32(self.duration.as_secs_f32())?;
        self.data.write(write, config)
    }

    fn read(read: &mut dyn Read, config: &C) -> io::Result<Self> {
        Ok(Self { 
            duration: Duration::from_secs_f32(read.read_f32()?), 
            data: D::read(read, config)?
        })
    }

}

impl<D: Codec<C>, C> SimpleElement<C> for ChallengeResponse<D> {
    const ID: u8 = id::CHALLENGE_RESPONSE;
    const LEN: ElementLength = ElementLength::Variable16;
}

/// Describe a challenge response for cuckoo cycle challenge type.
#[derive(Debug, Clone)]
pub struct CuckooCycleResponse {
    /// The full key used to initialize the Cuckoo Cycle context, this should start with
    /// the issued [`LoginChallenge::CuckooCycle::key_prefix`].
    pub key: Vec<u8>,
    /// Contains each solution found for the problem.
    pub solution: Vec<u32>,
}

impl SimpleCodec for CuckooCycleResponse {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_blob_variable(&self.key)?;
        for &nonce in &self.solution {
            write.write_u32(nonce)?;
        }
        Ok(())
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {

        let key = read.read_blob_variable()?;
        let mut solution = Vec::with_capacity(42);

        loop {
            solution.push(match read.read_u32() {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            });
        }
        
        Ok(CuckooCycleResponse { 
            key, 
            solution,
        })

    }

}
