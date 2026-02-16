use std::path::Path;
use unifiedlab::workflow::importer::DrawIoLoader;

#[test]
fn test_experiment_drawio() {
    let path = "experiment.drawio";
    assert!(Path::new(path).exists(), "experiment.drawio does not exist");

    let loader = DrawIoLoader::load_from_file(path).expect("Failed to load drawio file");

    // Check nodes
    let node_count = loader.graph.graph.node_count();
    println!("Node count: {}", node_count);
    assert!(node_count > 0, "No nodes found in graph");

    // Check edges
    let edge_count = loader.graph.graph.edge_count();
    println!("Edge count: {}", edge_count);
    assert!(edge_count > 0, "No edges found in graph");
}

#[test]
fn test_compressed_drawio() {
    let path = "compressed.drawio";
    assert!(Path::new(path).exists(), "compressed.drawio does not exist");

    let loader = DrawIoLoader::load_from_file(path).expect("Failed to load drawio file");

    // Check nodes
    let node_count = loader.graph.graph.node_count();
    println!("Node count: {}", node_count);
    assert!(
        node_count > 0,
        "No nodes found in graph for compressed file"
    );
}
