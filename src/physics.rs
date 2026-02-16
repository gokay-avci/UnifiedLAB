// src/physics.rs
//
// =============================================================================
// UNIFIEDLAB: SEMANTIC VALIDATOR (v 0.1 )
// =============================================================================
//
// The Gatekeeper.
// Uses KD-Trees for O(N log N) spatial checks.
//
// this module is a preparation step for structure aware recognition of the DAG propagation (i. e. to prevent unreasonable structures to populate the search)

use crate::core::Structure;
use anyhow::{anyhow, Result};
use kdtree::distance::squared_euclidean;
use kdtree::KdTree;

// ============================================================================
// 1. CONSTANTS & MASS TABLE
// ============================================================================

const CONVERSION_AMU_ANG_TO_G_CM3: f64 = 1.660539;

fn get_atomic_mass(symbol: &str) -> f64 {
    match symbol {
        "H" => 1.008,
        "He" => 4.0026,
        "Li" => 6.94,
        "Be" => 9.012,
        "B" => 10.81,
        "C" => 12.011,
        "N" => 14.007,
        "O" => 15.999,
        "F" => 18.998,
        "Ne" => 20.180,
        "Na" => 22.99,
        "Mg" => 24.305,
        "Al" => 26.982,
        "Si" => 28.085,
        "P" => 30.974,
        "S" => 32.06,
        "Cl" => 35.45,
        "K" => 39.098,
        "Ca" => 40.078,
        "Sc" => 44.956,
        "Ti" => 47.867,
        "V" => 50.942,
        "Cr" => 51.996,
        "Mn" => 54.938,
        "Fe" => 55.845,
        "Co" => 58.933,
        "Ni" => 58.693,
        "Cu" => 63.546,
        "Zn" => 65.38,
        "Ga" => 69.723,
        "Ge" => 72.63,
        "As" => 74.922,
        "Se" => 78.96,
        "Br" => 79.904,
        "Zr" => 91.224,
        "Mo" => 95.95,
        "Pd" => 106.42,
        "Ag" => 107.87,
        "Cd" => 112.41,
        "Sn" => 118.71,
        "Sb" => 121.76,
        "I" => 126.90,
        "Xe" => 131.29,
        "Ba" => 137.33,
        "La" => 138.91,
        "Ce" => 140.12,
        "W" => 183.84,
        "Pt" => 195.08,
        "Au" => 196.97,
        "Pb" => 207.2,
        _ => 100.0,
    }
}

// ============================================================================
// 2. THE TRAIT
// ============================================================================

pub trait SanityCheck {
    fn validate_physics(&self) -> Result<()>;
    fn check_overlaps(&self, min_dist: f64) -> Result<()>;
    fn check_density(&self) -> Result<()>;
}

// ============================================================================
// 3. IMPLEMENTATION
// ============================================================================

impl SanityCheck for Structure {
    fn validate_physics(&self) -> Result<()> {
        if let Some(lat) = &self.lattice {
            if lat.volume().abs() < 1e-3 {
                return Err(anyhow!(
                    "Lattice volume is near zero/degenerate: {:.4}",
                    lat.volume()
                ));
            }
        }

        self.check_overlaps(0.7)?;

        if self.lattice.is_some() {
            self.check_density()?;
        }

        Ok(())
    }

    fn check_overlaps(&self, min_dist: f64) -> Result<()> {
        let count = self.atoms.len();
        if count < 2 {
            return Ok(());
        }

        // Explicit types: <CoordinateType, ItemType, PointArray>
        let mut kdtree: KdTree<f64, usize, [f64; 3]> = KdTree::new(3);

        for (i, atom) in self.atoms.iter().enumerate() {
            kdtree
                .add(atom.position, i)
                .map_err(|e| anyhow!("KDTree error: {}", e))?;
        }

        let min_sq = min_dist * min_dist;

        for (i, atom) in self.atoms.iter().enumerate() {
            // Find 2 nearest: 1st is self (d=0), 2nd is neighbor
            let nearest = kdtree
                .nearest(&atom.position, 2, &squared_euclidean)
                .map_err(|e| anyhow!("KDTree query error: {}", e))?;

            if nearest.len() > 1 {
                let (dist_sq, &index) = nearest[1];

                // Edge case: Sometimes float precision makes self not the first result?
                // Logic: Check if 'index' is not 'i' and distance is too small.

                // Case A: Standard (0 is self, 1 is neighbor)
                if index != i && dist_sq < min_sq {
                    let atom_a = &self.atoms[i];
                    let atom_b = &self.atoms[index];
                    return Err(anyhow!(
                        "Atom overlap detected! {}[{}] and {}[{}] are {:.3}A apart.",
                        atom_a.symbol,
                        i,
                        atom_b.symbol,
                        index,
                        dist_sq.sqrt()
                    ));
                }

                // Case B: Self is stored weirdly or duplicates exist at 0.0 distance
                if index == i && nearest.len() > 2 {
                    let (d2, &idx2) = nearest[2];
                    if d2 < min_sq {
                        let atom_a = &self.atoms[i];
                        let atom_b = &self.atoms[idx2];
                        return Err(anyhow!(
                            "Atom overlap detected! {}[{}] and {}[{}] are {:.3}A apart.",
                            atom_a.symbol,
                            i,
                            atom_b.symbol,
                            idx2,
                            d2.sqrt()
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    fn check_density(&self) -> Result<()> {
        let lat = self
            .lattice
            .as_ref()
            .ok_or_else(|| anyhow!("Cannot check density: No lattice"))?;

        let volume = lat.volume();
        let mut total_mass = 0.0;

        for atom in &self.atoms {
            total_mass += get_atomic_mass(&atom.symbol);
        }

        let density = (total_mass / volume) * CONVERSION_AMU_ANG_TO_G_CM3;

        if density < 0.1 {
            return Err(anyhow!("Density too low: {:.3} g/cm3", density));
        }
        if density > 30.0 {
            return Err(anyhow!("Density too high: {:.3} g/cm3", density));
        }

        Ok(())
    }
}
