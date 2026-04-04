use clap::Parser;
use rust_blockchain::concept_registry::ConceptRegistry;
use rust_blockchain::hd::BitVec;

#[derive(Parser)]
#[command(author, version, about = "Concept registry tool")]
struct Cli {
    /// Path to registry file
    #[arg(short, long, default_value = "concept_registry.json")]
    registry: String,

    /// Developer identifier
    developer: String,

    /// Game identifier / namespace
    game: String,

    /// Concept name
    concept: String,

    /// Vector dimension
    #[arg(short, long, default_value_t = rust_blockchain::player_profile::profile_service::DEFAULT_DIM)]
    dim: usize,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    let mut registry = ConceptRegistry::load(&cli.registry)?;
    let key = format!("{}:{}:{}", cli.developer, cli.game, cli.concept);

    if registry.get(&key).is_some() {
        println!("Concept already exists in registry: {}", key);
    } else {
        let vec = BitVec::seed(&key, cli.dim);
        registry.insert(key.clone(), vec);
        registry.save(&cli.registry)?;
        println!("Added concept {} to registry", key);
    }
    if let Some(vec) = registry.get(&key) {
        println!("Vector lanes: {:?}", vec.lanes);
    }

    Ok(())
}
