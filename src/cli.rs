// TODO: `subject` command

// TODO: list names inside of .kdl file
// TODO: kindelia get name

// TODO: command to get number of txs on mempool

// TODO: way to check if transaction fits in a block
// TODO: publish to multiple nodes

// TODO: flag enable logging statements results (disabled by default)
// TODO: limit readback computational resources on aforementioned log and API calls

// TODO: flag to enable printing events (heartbeat) ?
// TODO: some way to pretty-print events (heartbeat) ?

use std::fmt;
use std::io::Read;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use warp::Future;

// use crate::api::{client as api_client, Hash, HexStatement};
// use crate::bits::ProtoSerialize;
// use crate::common::Name;
// use crate::{crypto};
// use crate::config;
// use crate::hvm::{self, view_statement, Statement};
// use crate::net;
// use crate::node;
// use crate::util::bytes_to_bitvec;

use kindelia::api::{client as api_client, Hash, HexStatement};
use kindelia::bits::ProtoSerialize;
use kindelia::common::Name;
use kindelia::crypto;
use kindelia::hvm::{self, view_statement, Statement};
use kindelia::net;
use kindelia::node;
use kindelia::util::bytes_to_bitvec;
use kindelia::{config, events};

// This client is meant to talk with a node implementing UDP protocol
// communication (the default)
type NC = UdpSocket;

/*

== Client ==

kindelia test file.kdl

kindelia serialize code.kdl > code.hex.txt

kindelia deserialize code.hex.txt
kindelia deserialize <<< a67bd36d75da

kindelia run-remote --hex <<< a67bd36d75da
kindelia publish    --hex <<< a67bd36d75da

kindelia sign code.hex.txt
kindelia sign <<< a67bd36d75da > code.sig.hex.tx

kindelia completion zsh >> .zshrc

== Remote ==

kindelia get fun Count code
kindelia get fun Count state
kindelia get fun Count slots

kindelia get reg Foo.Bar owner
kindelia get reg Foo.Bar list

kindelia get block 0xc7da4b76b4d7a64b7 | kindelia deserialize
kindelia get block 751
kindelia get block 2756

kindelia get ctr Pair code
kindelia get ctr Pair arity

kindelia get run <BLOCK_IDX> <STM_IDX>

kindelia get stats

kindelia get stats tick
kindelia get stats mana
kindelia get stats space
kindelia get stats ctr-count
kindelia get stats fun-count
kindelia get stats reg-count

kindelia [--api ""] run-remote  code.hex.txt
kindelia [--api ""] publish     code.hex.txt

== Node ==

kindelia node start --mine --local --log-events --nice-ui?
kindelia node clean [-f]       // asks confirmation

== Accounts ==

kindelia account ...

*/

fn run_on_remote<T, P, F>(
  api_url: &str,
  stmts: Vec<Statement>,
  f: F,
) -> Result<T, String>
where
  F: FnOnce(api_client::ApiClient, Vec<HexStatement>) -> P,
  P: Future<Output = Result<T, String>>,
{
  let stmts: Vec<HexStatement> = stmts.into_iter().map(|s| s.into()).collect();
  let client =
    api_client::ApiClient::new(api_url, None).map_err(|e| e.to_string())?;
  run_async_blocking(f(client, stmts))
}

// Clap CLI definitions
// ====================

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
  #[clap(subcommand)]
  command: CliCommand,
  #[clap(long, short = 'c')]
  /// Path to config file.
  config: Option<PathBuf>,
  /// Url to server host
  #[clap(long)]
  api: Option<String>,
}

#[derive(Subcommand)]
pub enum CliCommand {
  /// Test a Kindelia code file (.kdl), running locally.
  Test {
    /// The path to the file to test.
    file: FileInput,
    /// Whether to consider size and mana in the execution.
    #[clap(long)]
    sudo: bool,
  },
  /// Serialize a code file.
  Serialize {
    /// The path to the file to serialize.
    file: FileInput,
  },
  /// Deserialize a code file.
  Deserialize {
    /// The path to the file to deserialize.
    file: FileInput,
  },
  /// Deserialize a hex string of a encoded statement.
  Unserialize {
    /// Hex string of the serialized statement.
    stmt: String,
  },
  /// Sign a code file.
  Sign {
    /// The path to the file to sign.
    file: FileInput,
    /// File containing the 256-bit secret key, as a hex string
    #[clap(long, short = 's')]
    secret_file: PathBuf,
    #[clap(long, short = 'e')]
    encoded: bool,
    #[clap(long, short = 'E')]
    encoded_output: bool,
  },
  /// Test a Kindelia (.kdl) file, dry-running it on the current remote KVM state.
  RunRemote {
    /// Input file.
    file: FileInput,
    /// In case the input code is serialized.
    #[clap(long, short = 'e')]
    encoded: bool,
  },
  /// Post a Kindelia code file.
  Publish {
    /// The path to the file to post.
    file: FileInput,
    /// In case the input code is serialized.
    #[clap(long, short = 'e')]
    encoded: bool,
  },
  // Post a (serialized) statement
  Post {
    /// Hex string of the serialized statement.
    stmt: String,
  },
  /// Get remote information.
  Get {
    /// The kind of information to get.
    #[clap(subcommand)]
    kind: GetKind,
    /// Outputs JSON machine readable output.
    #[clap(long, short)]
    json: bool,
  },
  /// Initialize the configuration file.
  Init,
  /// Node commands.
  Node {
    /// Which command run.
    #[clap(subcommand)]
    command: NodeCommand,
    /// Base path to store the node's data in.
    #[clap(long)]
    data_dir: Option<PathBuf>,
  },
  /// Generate auto-completion for a shell.
  Completion {
    /// The shell to generate completion for.
    shell: String,
  },
  Util {
    /// Which command run.
    #[clap(subcommand)]
    command: UtilCommand,
  },
}

#[derive(Subcommand)]
pub enum NodeCommand {
  /// Clean the node's data.
  Clean,
  /// Starts a Kindelia node.
  Start {
    /// Network id / magic number.
    #[clap(long)]
    network_id: Option<u64>,
    /// Initial peer nodes.
    #[clap(long, short = 'p')]
    initial_peers: Option<Vec<String>>,
    /// Mine blocks.
    #[clap(long, short = 'm')]
    mine: bool,
    /// Log events as JSON
    #[clap(long, short)]
    json: bool,
  },
}

#[derive(Subcommand)]
pub enum UtilCommand {
  /// Generate a new keypair.
  DecodeName { file: FileInput },
}

#[derive(Subcommand)]
pub enum GetKind {
  /// Get a constructor by name.
  Ctr {
    /// The name of the constructor to get.
    name: Name,
    /// The stat of the constructor to get.
    #[clap(subcommand)]
    stat: GetCtrKind,
  },
  /// [NOT IMPLEMENTED] Get a block by hash.
  Block {
    /// The hash of the block to get.
    hash: String,
  },
  /// Get a function by name.
  Fun {
    /// The name of the function to get.
    name: Name,
    /// The stat of the function to get.
    #[clap(subcommand)]
    stat: GetFunKind,
  },
  /// Get a registered namespace by name.
  Reg {
    /// The name of the namespace to get.
    name: String,
    /// The stat of the namespace to get.
    #[clap(subcommand)]
    stat: GetRegKind,
  },
  BlockHash {
    index: u64,
  },
  /// Get node stats.
  Stats {
    /// The stat of the node to get.
    #[clap(subcommand)]
    stat_kind: Option<GetStatsKind>,
  },
  Peers {
    /// Get all seen peers, including inactive ones
    #[clap(long)]
    all: bool,
  },
}

#[derive(Subcommand)]
pub enum GetFunKind {
  /// Get the code of a function.
  Code,
  /// Get the state of a function.
  State,
  /// Get the slots of a function.
  Slots,
}

#[derive(Subcommand)]
pub enum GetRegKind {
  /// Get the owner of a namespace.
  Owner,
  /// Get the list of statements in a namespace.
  List,
}

#[derive(Subcommand)]
pub enum GetCtrKind {
  /// Get the code of a constructor.
  Code,
  /// Get the arity of a constructor.
  Arity,
}

#[derive(Subcommand)]
pub enum GetStatsKind {
  /// Get the tick (tip block height).
  Tick,
  /// Get the used mana.
  Mana,
  /// Get the quantity of used space.
  // TODO: we should measure this as slots/nodes/cells, not bits
  Space,
  /// Get the number of functions.
  FunCount,
  /// Get the number of constructors.
  CtrCount,
  /// Get the number of namespaces.
  RegCount,
}

// Config Resolution
// =================

#[derive(derive_builder::Builder)]
#[builder(setter(strip_option))]
struct ConfigSettings<T, F>
where
  T: Clone + Sized,
  F: Fn() -> Result<T, String>,
{
  #[builder(default)]
  env: Option<&'static str>,
  prop: Option<&'static str>,
  default_value: F,
}

impl<T, F> ConfigSettings<T, F>
where
  T: Clone + Sized,
  F: Fn() -> Result<T, String>,
{
  /// Resolve config value.
  ///
  /// Priority is:
  /// 1. CLI argument
  /// 2. Environment variable
  /// 3. Config file
  /// 4. Default value
  pub fn resolve(
    self,
    cli_value: Option<T>,
    config_values: Option<&toml::Value>,
  ) -> Result<T, String>
  where
    T: ArgumentFrom<String> + ArgumentFrom<toml::Value>,
  {
    if let Some(value) = cli_value {
      // Read from CLI argument
      return Ok(value);
    }
    if let Some(Ok(env_value)) = self.env.map(std::env::var) {
      // If env var is set, read from it
      return T::arg_from(env_value);
    }
    if let (Some(prop_path), Some(config_values)) = (self.prop, config_values) {
      // If config file and argument prop path are set, read from config file
      return Self::resolve_from_config_aux(config_values, prop_path);
    }
    (self.default_value)()
  }

  // TODO: refactor

  fn resolve_from_file_only(
    self,
    config_values: Option<&toml::Value>,
  ) -> Result<T, String>
  where
    T: ArgumentFrom<toml::Value>,
  {
    if let Some(prop_path) = self.prop {
      if let Some(config_values) = config_values {
        Self::resolve_from_config_aux(config_values, prop_path)
      } else {
        (self.default_value)()
      }
    } else {
      panic!("Cannot resolve from config file config without 'prop' field set")
    }
  }

  fn resolve_from_file_opt(
    self,
    config_values: Option<&toml::Value>,
  ) -> Result<Option<T>, String>
  where
    T: ArgumentFrom<toml::Value>,
  {
    if let Some(prop_path) = self.prop {
      if let Some(config_values) = config_values {
        let value = Self::get_prop(config_values, prop_path);
        if let Some(value) = value {
          return T::arg_from(value).map(|v| Some(v)).map_err(|e| {
            format!(
              "Could not convert value of '{}' into desired type: {}",
              prop_path, e
            )
          });
        }
      }
      Ok(None)
    } else {
      panic!("Cannot resolve from config file config without 'prop' field set")
    }
  }

  fn resolve_from_config_aux(
    config_values: &toml::Value,
    prop_path: &str,
  ) -> Result<T, String>
  where
    T: ArgumentFrom<toml::Value>,
  {
    let value = Self::get_prop(config_values, prop_path)
      .ok_or(format!("Could not found prop '{}' in config file.", prop_path))?;
    T::arg_from(value).map_err(|e| {
      format!(
        "Could not convert value of '{}' into desired type: {}",
        prop_path, e
      )
    })
  }

  fn get_prop(mut value: &toml::Value, prop_path: &str) -> Option<toml::Value> {
    // Doing this way because of issue #469 toml-rs
    let props: Vec<_> = prop_path.split('.').collect();
    for prop in props {
      value = value.get(&prop)?;
    }
    Some(value.clone())
  }
}

// Macros
// ======

macro_rules! resolve_cfg {
  (env = $env:expr, prop = $prop:expr, default = $default:expr, val = $cli:expr, cfg = $cfg:expr $(,)*) => {
    ConfigSettingsBuilder::default()
      .env($env)
      .prop($prop)
      .default_value(|| Ok($default))
      .build()
      .unwrap()
      .resolve($cli, $cfg)?
  };
  (env = $env:expr, prop = $prop:expr, no_default = $default:expr, val = $cli:expr, cfg = $cfg:expr $(,)*) => {
    ConfigSettingsBuilder::default()
      .env($env)
      .prop($prop)
      .default_value(|| Err($default))
      .build()
      .unwrap()
      .resolve($cli, $cfg)?
  };
}

// CLI main function
// =================

// TODO: refactor into main?

/// Parse Cli arguments and do an action
pub fn run_cli() -> Result<(), String> {
  let parsed = Cli::parse();
  let default_kindelia_path = || {
    let home_dir = dirs::home_dir().ok_or("Could not find $HOME")?;
    Ok(home_dir.join(".kindelia"))
  };

  let default_config_path = || {
    let kindelia_path = default_kindelia_path()?;
    Ok(kindelia_path.join("kindelia.toml"))
  };

  // get possible config path and content
  let config_path = ConfigSettings {
    env: Some("KINDELIA_CONFIG"),
    prop: None,
    default_value: default_config_path,
  }
  .resolve(parsed.config, None)?;

  let api_url = ConfigSettings {
    env: Some("KINDELIA_API_URL"),
    prop: None,
    default_value: || Ok("http://localhost:8000".to_string()),
  }
  .resolve(parsed.api, None)?;

  match parsed.command {
    CliCommand::Test { file, sudo } => {
      let code: String = file.read_to_string()?;
      test_code(&code, sudo);
      Ok(())
    }
    CliCommand::Serialize { file } => {
      let code: String = file.read_to_string()?;
      serialize_code(&code);
      Ok(())
    }
    CliCommand::Deserialize { file } => {
      let code: String = file.read_to_string()?;
      deserialize_code(&code)
    }
    CliCommand::Unserialize { stmt } => deserialize_code(&stmt),
    CliCommand::Sign { file, secret_file, encoded, encoded_output } => {
      let skey: String = arg_from_file_or_stdin(secret_file.into())?;
      let skey = skey.trim();
      let skey = hex::decode(skey).map_err(|err| {
        format!("Secret key should be valid hex string: {}", err)
      })?;
      let skey: [u8; 32] = skey
        .try_into()
        .map_err(|_| "Secret key should have exactly 64 bytes".to_string())?;
      let code = load_code(file, encoded)?;
      let statement = match &code[..] {
        [stmt] => sign_code(stmt, &skey),
        _ => Err("Input file should contain exactly one statement".to_string()),
      }?;
      if encoded_output {
        println!("{}", hex::encode(statement.proto_serialized().to_bytes()));
      } else {
        println!("{}", view_statement(&statement));
      };
      Ok(())
    }
    CliCommand::RunRemote { file, encoded } => {
      // TODO: client timeout
      let code = file.read_to_string()?;
      let f = |client: api_client::ApiClient, stmts| async move {
        client.run_code(stmts).await
      };
      let stmts = if encoded {
        statements_from_hex_seq(&code)?
      } else {
        hvm::parse_code(&code)?
      };
      let results = run_on_remote(&api_url, stmts, f)?;
      for result in results {
        println!("{}", result);
      }
      Ok(())
    }
    CliCommand::Publish { file, encoded } => {
      let code = file.read_to_string()?;
      let stmts = if encoded {
        statements_from_hex_seq(&code)?
      } else {
        hvm::parse_code(&code)?
      };
      publish_code(&api_url, stmts)
    }
    CliCommand::Post { stmt } => {
      let stmts = statements_from_hex_seq(&stmt)?;
      publish_code(&api_url, stmts)
    }
    CliCommand::Get { kind, json } => {
      let prom = get_info(kind, json, &api_url);
      run_async_blocking(prom)
    }
    CliCommand::Init => {
      let path = default_config_path()?;
      eprintln!("Writing default configuration to '{}'...", path.display());
      init_config_file(&path)?;
      Ok(())
    }
    CliCommand::Node { command, data_dir } => {
      let config = handle_config_file(&config_path)?;
      let config = Some(&config);

      let data_path = ConfigSettings {
        env: Some("KINDELIA_NODE_DATA_DIR"),
        prop: Some("node.data.dir"),
        default_value: default_kindelia_path,
      }
      .resolve(data_dir, config)?;

      match command {
        NodeCommand::Clean => {
          // warning
          println!(
            "WARNING! This will delete all the files present in '{}'...",
            data_path.display()
          );
          // confirmation
          println!("Do you want to continue? ['y' for YES / or else NO]");
          let mut answer = String::new();
          std::io::stdin()
            .read_line(&mut answer)
            .map_err(|err| format!("Could not read your answer: '{}'", err))?;
          // only accept 'y' as positive answer, anything else will be ignored
          if answer.trim().to_lowercase() == "y" {
            std::fs::remove_dir_all(data_path).map_err(|err| {
              format!("Could not remove the files: '{}'", err)
            })?;
            println!("All items were removed.");
          } else {
            println!("Canceling operation.");
          }
          Ok(())
        }
        NodeCommand::Start { initial_peers, network_id, mine, json } => {
          // TODO: refactor config resolution out of command handling (how?)

          // Get arguments from cli, env or config

          let network_id = resolve_cfg!(
            env = "KINDELIA_NETWORK_ID",
            prop = "node.network.network_id",
            no_default = "Missing `network_id` paramenter.".to_string(),
            val = network_id,
            cfg = config,
          );

          let initial_peers = resolve_cfg!(
            env = "KINDELIA_NODE_INITIAL_PEERS",
            prop = "node.network.initial_peers",
            default = vec![],
            val = initial_peers,
            cfg = config,
          );

          let mine = resolve_cfg!(
            env = "KINDELIA_MINE",
            prop = "node.mining.enable",
            default = false,
            val = flag_to_option(mine),
            cfg = config,
          );

          let slow_mining = ConfigSettingsBuilder::default()
            .env("KINDELIA_SLOW_MINING")
            .prop("node.debug.slow_mining")
            .default_value(|| Ok(0))
            .build()
            .unwrap()
            .resolve_from_file_opt(config)?;

          let api_config = ConfigSettingsBuilder::default()
            .prop("node.api")
            .default_value(|| Ok(config::ApiConfig::default()))
            .build()
            .unwrap()
            .resolve_from_file_only(config)?;

          // Start
          let node_comm = init_socket().expect("Could not open a UDP socket");
          let initial_peers = initial_peers
            .iter()
            .map(|x| net::parse_address(x))
            .collect::<Vec<_>>();

          let node_cfg = config::NodeConfig {
            network_id,
            data_path,
            mining: config::MineConfig { enabled: mine, slow_mining },
            ui: Some(config::UiConfig {
              json,
              tags: [events::NodeEventDiscriminant::Heartbeat].to_vec(),
            }),
            api: Some(api_config),
            ws: None, // TODO: load from config file
          };

          node::start(node_cfg, node_comm, initial_peers);

          Ok(())
        }
      }
    }
    CliCommand::Util { command } => match command {
      UtilCommand::DecodeName { file } => {
        let txt = file.read_to_string()?;
        let data: Result<Vec<Vec<u8>>, _> = txt
          .trim()
          .split(|c: char| c.is_whitespace())
          .map(hex::decode)
          .collect();
        let data =
          data.map_err(|err| format!("Invalid hex string: {}", err))?;
        let nums = data.iter().map(|v| bytes_to_u128(v));
        for num in nums {
          if let Some(num) = num {
            if let Ok(name) = Name::try_from(num) {
              println!("{}", name);
              continue;
            }
          }
          println!();
        }
        Ok(())
      }
    },
    CliCommand::Completion { .. } => todo!(),
  }
}

// Main Actions
// ============

pub async fn get_info(
  kind: GetKind,
  json: bool,
  host_url: &str,
) -> Result<(), String> {
  let client =
    api_client::ApiClient::new(host_url, None).map_err(|e| e.to_string())?;
  match kind {
    GetKind::BlockHash { index } => {
      let block_hash = client.get_block_hash(index).await?;
      println!("{}", block_hash);
      Ok(())
    }
    GetKind::Block { hash } => {
      let hash = Hash::try_from(hash.as_str())?;
      let block = client.get_block(hash).await?;
      println!("{:#?}", block);
      Ok(())
    }
    GetKind::Ctr { name, stat } => {
      let ctr_info = client.get_constructor(name).await?;
      match stat {
        GetCtrKind::Arity => {
          println!("{}", ctr_info.arit)
        }
        GetCtrKind::Code => {
          let args = (0..ctr_info.arit)
            .map(|x| format!("x{}", x))
            .collect::<Vec<_>>()
            .join(" ");
          println!("{{{} {}}}", name, args)
        }
      }
      Ok(())
    }
    GetKind::Fun { name, stat } => match stat {
      GetFunKind::Code => {
        let func_info = client.get_function(name).await?;
        if json {
          println!("{}", serde_json::to_string(&func_info).unwrap());
        } else {
          let func = func_info.func;
          let statement = hvm::Statement::Fun {
            name,
            args: vec![Name::NONE],
            func,
            init: hvm::Term::var(Name::NONE),
            sign: None,
          };
          println!("{}", statement);
        }
        Ok(())
      }
      GetFunKind::State => {
        let state = client.get_function_state(name).await?;
        if json {
          println!("{}", serde_json::to_string_pretty(&state).unwrap());
        } else {
          println!("{}", state);
        }
        Ok(())
      }
      GetFunKind::Slots => todo!(),
    },
    GetKind::Reg { name, stat } => {
      let reg_info = client.get_reg_info(&name).await?;
      match stat {
        GetRegKind::Owner => {
          println!("{:x}", *(reg_info.ownr))
        }
        GetRegKind::List => {
          for name in reg_info.stmt {
            println!("{}", name)
          }
        }
      }
      Ok(())
    }
    GetKind::Stats { stat_kind } => {
      let stats = client.get_stats().await?;
      match stat_kind {
        None => {
          if json {
            println!("{}", serde_json::to_string_pretty(&stats).unwrap());
          } else {
            println!("{:#?}", stats);
          }
        }
        Some(stat_kind) => {
          let val = match stat_kind {
            GetStatsKind::Tick => stats.tick,
            GetStatsKind::Mana => stats.mana,
            GetStatsKind::Space => stats.space,
            GetStatsKind::FunCount => stats.fun_count,
            GetStatsKind::CtrCount => stats.ctr_count,
            GetStatsKind::RegCount => stats.reg_count,
          };
          println!("{}", val);
        }
      };
      Ok(())
    }
    GetKind::Peers { all } => {
      let peers = client.get_peers::<NC>(all).await?;
      for peer in peers {
        println!("{}", peer.address)
      }
      Ok(())
    }
  }
}

pub fn serialize_code(code: &str) {
  let statements =
    hvm::read_statements(code).map_err(|err| err.erro).unwrap().1;
  for statement in statements {
    println!("{}", hex::encode(statement.proto_serialized().to_bytes()));
  }
}

pub fn deserialize_code(content: &str) -> Result<(), String> {
  let statements = statements_from_hex_seq(content)?;
  for statement in statements {
    println!("{}", view_statement(&statement))
  }
  Ok(())
}

// TODO: should not open file
pub fn sign_code(
  statement: &Statement,
  skey: &[u8; 32],
) -> Result<Statement, String> {
  let user = crypto::Account::from_private_key(skey);
  let hash = hvm::hash_statement(statement);
  let sign = user.sign(&hash);
  match statement {
    Statement::Fun { sign, .. }
    | Statement::Ctr { sign, .. }
    | Statement::Run { sign, .. }
    | Statement::Reg { sign, .. } => {
      if sign.is_some() {
        return Err("Statement already has a signature.".to_string());
      }
    }
  };
  let stat = hvm::set_sign(statement, sign);
  Ok(stat)
}

pub fn publish_code(
  api_url: &str,
  stmts: Vec<Statement>,
) -> Result<(), String> {
  let f = |client: api_client::ApiClient, stmts| async move {
    client.publish_code(stmts).await
  };
  let results = run_on_remote(api_url, stmts, f)?;
  for (i, result) in results.iter().enumerate() {
    print!("Transaction #{}: ", i);
    match result {
      Ok(_) => println!("PUBLISHED (tx added to mempool)"),
      Err(_) => {
        println!("NOT PUBLISHED (tx is probably already on mempool)")
      }
    }
  }
  Ok(())
}

pub fn test_code(code: &str, sudo: bool) {
  hvm::test_statements_from_code(code, sudo);
}

fn init_socket() -> Option<UdpSocket> {
  let try_ports =
    [net::UDP_PORT, net::UDP_PORT + 1, net::UDP_PORT + 2, net::UDP_PORT + 3];
  for port in try_ports {
    if let Ok(socket) = UdpSocket::bind(&format!("0.0.0.0:{}", port)) {
      socket.set_nonblocking(true).ok();
      return Some(socket);
    }
  }
  None
}

// Utils
// =====

pub fn flag_to_option(flag: bool) -> Option<bool> {
  if flag {
    Some(true)
  } else {
    None
  }
}

pub fn bytes_to_u128(bytes: &[u8]) -> Option<u128> {
  let mut num: u128 = 0;
  for byte in bytes {
    num = num.checked_shl(8)?;
    num += *byte as u128;
  }
  Some(num)
}

// Async
// -----

fn run_async_blocking<T, E: ToString, P>(prom: P) -> Result<T, E>
where
  P: Future<Output = Result<T, E>>,
{
  let runtime = tokio::runtime::Runtime::new().unwrap();
  runtime.block_on(prom)
}

// Config
// ------

fn handle_config_file(path: &Path) -> Result<toml::Value, String> {
  if !path.exists() {
    eprintln!("WARNING: Config file not found. Default config file will be created on '{}'...\n", path.display());
    init_config_file(path)?;
    thread::sleep(Duration::from_millis(5000));
  }
  let content = std::fs::read_to_string(path).map_err(|e| {
    format!("Error reading config file from '{}': {}", path.display(), e)
  })?;
  let config = content.parse::<toml::Value>().map_err(|e| {
    format!("Error parsing config file from '{}': {}", path.display(), e)
  })?;
  Ok(config)
}

fn init_config_file(path: &Path) -> Result<(), String> {
  let dir_path = path.parent().ok_or_else(|| {
    format!("Failed to resolve parent directory for '{}'", path.display())
  })?;
  let default_content = include_str!("../default.toml");
  std::fs::create_dir_all(&dir_path).map_err(|e| {
    format!("Could not create '{}' directory: {}", dir_path.display(), e)
  })?;
  std::fs::write(path, default_content)
    .map_err(|e| format!("Could not write to '{}': {}", path.display(), e))
}

// Code
// ----

fn load_code(file: FileInput, encoded: bool) -> Result<Vec<Statement>, String> {
  let code = file.read_to_string()?;
  handle_code(&code, encoded)
}

fn handle_code(code: &str, encoded: bool) -> Result<Vec<Statement>, String> {
  if encoded {
    statements_from_hex_seq(code)
  } else {
    hvm::parse_code(code)
  }
}

fn statements_from_hex_seq(txt: &str) -> Result<Vec<Statement>, String> {
  txt
    .trim()
    .split(|c: char| c.is_whitespace())
    .map(statement_from_hex)
    .collect()
}

fn statement_from_hex(hex: &str) -> Result<Statement, String> {
  let bytes = hex::decode(hex)
    .map_err(|err| format!("Invalid hexadecimal '{}': {}", hex, err))?;
  hvm::Statement::proto_deserialized(&bytes_to_bitvec(&bytes))
    .ok_or(format!("Failed to deserialize '{}'", hex))
}

fn arg_from_file_or_stdin<T: ArgumentFrom<String>>(
  file: FileInput,
) -> Result<T, String> {
  match file {
    FileInput::Path { path } => {
      // read from file
      let content = std::fs::read_to_string(&path).map_err(|err| {
        format!("Cannot read from '{:?}' file: {}", path, err)
      })?;
      T::arg_from(content)
    }
    FileInput::Stdin => {
      // read from stdin
      let mut input = String::new();
      match std::io::stdin().read_line(&mut input) {
        Ok(_) => T::arg_from(input.trim().to_string()),
        Err(err) => Err(format!("Could not read from stdin: {}", err)),
      }
    }
  }
}

// Auxiliar traits and types
// =========================

// FileInput
// ---------

/// Represents input from a file or stdin.
#[derive(Debug)]
pub enum FileInput {
  Stdin,
  Path { path: PathBuf },
}

impl From<PathBuf> for FileInput {
  fn from(path: PathBuf) -> Self {
    FileInput::Path { path }
  }
}

impl FromStr for FileInput {
  type Err = std::convert::Infallible;
  fn from_str(txt: &str) -> Result<Self, Self::Err> {
    let val = if txt == "-" {
      Self::Stdin
    } else {
      let path = txt.into();
      Self::Path { path }
    };
    Ok(val)
  }
}

impl fmt::Display for FileInput {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Path { path } => write!(f, "{}", path.display()),
      Self::Stdin => write!(f, "<stdin>"),
    }
  }
}

// TODO: alternative that do not read the whole file immediately
impl FileInput {
  fn read_to_string(&self) -> Result<String, String> {
    match self {
      FileInput::Path { path } => {
        // read from file
        std::fs::read_to_string(&path)
          .map_err(|e| format!("Cannot read from '{:?}' file: {}", path, e))
      }
      FileInput::Stdin => {
        // read from stdin
        let mut buff = String::new();
        std::io::stdin()
          .read_to_string(&mut buff)
          .map_err(|e| format!("Could not read from stdin: {}", e))?;
        Ok(buff)
      }
    }
  }
}

// ArgumentFrom
// ------------

/// A trait to convert from anything to a type T.
/// It is equal to standard From trait, but
/// it has the From<String> for Vec<String> implementation.
/// As like From, the conversion must be perfect.
pub trait ArgumentFrom<T>: Sized {
  fn arg_from(value: T) -> Result<Self, String>;
}

impl ArgumentFrom<String> for String {
  fn arg_from(t: String) -> Result<Self, String> {
    Ok(t)
  }
}

impl ArgumentFrom<String> for u64 {
  fn arg_from(t: String) -> Result<Self, String> {
    t.parse().map_err(|e| format!("Invalid integer: `{}`", e))
  }
}

impl ArgumentFrom<toml::Value> for u64 {
  fn arg_from(value: toml::Value) -> Result<Self, String> {
    match value {
      toml::Value::Integer(i) => Ok(i as u64),
      toml::Value::String(s) => {
        let s = s.trim_start_matches("0x");
        let num = u64::from_str_radix(s, 16)
          .map_err(|e| format!("Invalid hexadecimal '{}': {}", s, e))?;
        Ok(num)
      }
      _ => Err(format!("Invalid integer '{}'", value)),
    }
  }
}

impl ArgumentFrom<String> for Vec<String> {
  fn arg_from(t: String) -> Result<Self, String> {
    Ok(t.split(',').map(|x| x.to_string()).collect())
  }
}

impl ArgumentFrom<String> for bool {
  fn arg_from(t: String) -> Result<Self, String> {
    if t == "true" {
      Ok(true)
    } else if t == "false" {
      Ok(false)
    } else {
      Err(format!("Invalid boolean value: {}", t))
    }
  }
}

impl ArgumentFrom<String> for PathBuf {
  fn arg_from(t: String) -> Result<Self, String> {
    if let Some(path) = t.strip_prefix("~/") {
      let home_dir =
        dirs::home_dir().ok_or("Could not find $HOME directory.")?;
      Ok(home_dir.join(path))
    } else {
      PathBuf::from_str(&t).map_err(|_| format!("Invalid path: {}", t))
    }
  }
}

impl ArgumentFrom<toml::Value> for PathBuf {
  fn arg_from(value: toml::Value) -> Result<Self, String> {
    let t: String =
      value.try_into().map_err(|_| "Could not convert value to PahtBuf")?;
    PathBuf::arg_from(t)
  }
}

impl ArgumentFrom<toml::Value> for String {
  fn arg_from(t: toml::Value) -> Result<Self, String> {
    t.try_into().map_err(|_| "Could not convert value into String".to_string())
  }
}

impl ArgumentFrom<toml::Value> for Vec<String> {
  fn arg_from(t: toml::Value) -> Result<Self, String> {
    t.try_into().map_err(|_| "Could not convert value into array".to_string())
  }
}

impl ArgumentFrom<toml::Value> for bool {
  fn arg_from(t: toml::Value) -> Result<Self, String> {
    t.as_bool().ok_or(format!("Invalid boolean value: {}", t))
  }
}

impl ArgumentFrom<toml::Value> for config::ApiConfig {
  fn arg_from(t: toml::Value) -> Result<Self, String> {
    t.try_into().map_err(|_| "Could not convert value into array".to_string())
  }
}
