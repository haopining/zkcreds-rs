use crate::{
    attrs::Attrs,
    identity_crh::{IdentityCRH, IdentityCRHGadget, UnitVar},
    proof_data_structures::{TreeProof, TreeProvingKey},
    sparse_merkle::{constraints::SparseMerkleTreePathVar, SparseMerkleTree, SparseMerkleTreePath},
};

use core::marker::PhantomData;
use std::collections::BTreeMap;

use ark_crypto_primitives::{
    commitment::{constraints::CommitmentGadget, CommitmentScheme},
    crh::{constraints::TwoToOneCRHGadget, TwoToOneCRH},
    merkle_tree::{Config as TreeConfig, TwoToOneParam},
};
use ark_ec::PairingEngine;
use ark_ff::to_bytes;
use ark_ff::{PrimeField, ToConstraintField};
use ark_r1cs_std::alloc::AllocVar;
use ark_relations::{
    ns,
    r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError},
};
use ark_std::rand::Rng;

#[cfg(test)]
use crate::proof_data_structures::TreeVerifyingKey;

/// A sparse Merkle tree config which uses the identity function for leaf hashes (we don't need to
/// hash commitments)
pub struct ComTreeConfig<H: TwoToOneCRH>(H);

impl<H: TwoToOneCRH> TreeConfig for ComTreeConfig<H> {
    type LeafHash = IdentityCRH;
    type TwoToOneHash = H;
}

/// A Merkle tree of attribute commitments
pub struct ComTree<ConstraintF, H, AC>
where
    ConstraintF: PrimeField,
    H: TwoToOneCRH,
    H::Output: ToConstraintField<ConstraintF>,
    AC: CommitmentScheme,
    AC::Output: ToConstraintField<ConstraintF>,
{
    /// The tree's contents
    tree: SparseMerkleTree<ComTreeConfig<H>>,

    _marker: PhantomData<(ConstraintF, AC)>,
}

impl<ConstraintF, H, AC> ComTree<ConstraintF, H, AC>
where
    ConstraintF: PrimeField,
    H: TwoToOneCRH,
    H::Output: ToConstraintField<ConstraintF>,
    AC: CommitmentScheme,
    AC::Output: ToConstraintField<ConstraintF>,
{
    /// Returns this tree's root
    pub fn root(&self) -> H::Output {
        self.tree.root()
    }

    /// Makes an empty list with capacity `2^tree_height`. Height MUST be at least 2.
    pub fn empty(crh_params: H::Parameters, tree_height: u32) -> ComTree<ConstraintF, H, AC> {
        ComTree {
            tree: SparseMerkleTree::empty::<AC::Output>((), crh_params, tree_height),
            _marker: PhantomData,
        }
    }

    /// Makes a list of capacity `2^tree_height` that's populated with all the given commitments
    /// at the given indices. Height MUST be at least 2.
    ///
    /// Panics
    /// =====
    /// Panics if any key in `coms` is greater than or equal to `2^tree_height`
    pub fn new(
        crh_params: H::Parameters,
        tree_height: u32,
        coms: &BTreeMap<u64, AC::Output>,
    ) -> ComTree<ConstraintF, H, AC> {
        let tree = SparseMerkleTree::new::<AC::Output>((), crh_params, tree_height, coms)
            .expect("could not instantiate ComTree");

        ComTree {
            tree,
            _marker: PhantomData,
        }
    }

    /// Inserts a commitment at index `idx`. This will overwrite the existing entry if there is one.
    ///
    /// Panics
    /// =====
    /// Panics when `idx >= 2^log_capacity`
    pub fn insert(&mut self, idx: u64, com: &AC::Output) {
        self.tree.insert(idx, com).expect("could not insert item");
    }

    /// Removes the entry at index `idx`, if one exists
    ///
    /// Panics
    /// =====
    /// Panics when `idx >= 2^tree_height`
    pub fn remove(&mut self, idx: u64) {
        self.tree.remove(idx).expect("could not remove item");
    }

    /// Proves that the given attribute commitment is at the specified tree index
    pub fn prove_membership<R, E, A, ACG, HG>(
        &self,
        rng: &mut R,
        pk: &TreeProvingKey<E, A, AC, ACG, H, HG>,
        idx: u64,
        attrs_com: AC::Output,
    ) -> Result<TreeProof<E, A, AC, ACG, H, HG>, SynthesisError>
    where
        R: Rng,
        E: PairingEngine<Fr = ConstraintF>,
        A: Attrs<E::Fr, AC>,
        ACG: CommitmentGadget<AC, E::Fr>,
        HG: TwoToOneCRHGadget<H, E::Fr>,
    {
        // Get the root, its commitment, and the auth path of the given idx
        let root = self.tree.root();
        let auth_path = self
            .tree
            .generate_proof(idx, &attrs_com)
            .expect("could not construct auth path");

        // Construct the prover with all the relevant info, and prove
        let prover: TreeMembershipProver<E::Fr, AC, ACG, H, HG> = TreeMembershipProver {
            height: self.tree.height,
            crh_param: self.tree.two_to_one_param.clone(),
            attrs_com,
            root,
            auth_path: Some(auth_path),
            _marker: PhantomData,
        };

        let proof = ark_groth16::create_random_proof(prover, &pk.pk, rng)?;
        Ok(TreeProof {
            proof,
            _marker: PhantomData,
        })
    }
}

/// Generates the membership proving key for this tree
pub fn gen_tree_memb_crs<R, E, A, AC, ACG, H, HG>(
    rng: &mut R,
    crh_param: H::Parameters,
    height: u32,
) -> Result<TreeProvingKey<E, A, AC, ACG, H, HG>, SynthesisError>
where
    R: Rng,
    E: PairingEngine,
    A: Attrs<E::Fr, AC>,
    AC: CommitmentScheme,
    AC::Output: ToConstraintField<E::Fr>,
    ACG: CommitmentGadget<AC, E::Fr>,
    H: TwoToOneCRH,
    H::Output: ToConstraintField<E::Fr>,
    HG: TwoToOneCRHGadget<H, E::Fr>,
{
    let prover: TreeMembershipProver<E::Fr, AC, ACG, H, HG> = TreeMembershipProver {
        height,
        crh_param,
        attrs_com: Default::default(),
        root: Default::default(),
        //root: self.tree.root(),
        //root_com_nonce: self.nonce,
        auth_path: None,
        _marker: PhantomData,
    };
    let pk = ark_groth16::generate_random_parameters(prover, rng)?;
    Ok(TreeProvingKey {
        pk,
        _marker: PhantomData,
    })
}

#[cfg(test)]
pub(crate) fn verify_tree_memb<E, A, AC, ACG, H, HG>(
    vk: &TreeVerifyingKey<E, A, AC, ACG, H, HG>,
    proof: &TreeProof<E, A, AC, ACG, H, HG>,
    attrs_com: &AC::Output,
    merkle_root: &H::Output,
) -> Result<bool, SynthesisError>
where
    E: PairingEngine,
    A: Attrs<E::Fr, AC>,
    AC: CommitmentScheme,
    ACG: CommitmentGadget<AC, E::Fr>,
    AC::Output: ToConstraintField<E::Fr>,
    H: TwoToOneCRH,
    H::Output: ToConstraintField<E::Fr>,
    HG: TwoToOneCRHGadget<H, E::Fr>,
{
    let attr_com_input = attrs_com.to_field_elements().unwrap();
    let root_input = merkle_root.to_field_elements().unwrap();

    let all_inputs = [attr_com_input, root_input].concat();
    ark_groth16::verify_proof(&vk.pvk, &proof.proof, &all_inputs)
}

/// A circuit that proves that a commitment to `attrs` appears in the Merkle tree of height `height`
/// defined by root hash `root`.
struct TreeMembershipProver<ConstraintF, AC, ACG, H, HG>
where
    ConstraintF: PrimeField,
    AC: CommitmentScheme,
    AC::Output: ToConstraintField<ConstraintF>,
    ACG: CommitmentGadget<AC, ConstraintF>,
    H: TwoToOneCRH,
    H::Output: ToConstraintField<ConstraintF>,
    HG: TwoToOneCRHGadget<H, ConstraintF>,
{
    // Constants //
    height: u32,
    crh_param: TwoToOneParam<ComTreeConfig<H>>,

    // Private inputs //
    /// The leaf value
    attrs_com: AC::Output,
    /// The tree root's value
    root: H::Output,
    /// Merkle auth path of the leaf `attrs_com`
    auth_path: Option<SparseMerkleTreePath<ComTreeConfig<H>>>,

    // Marker //
    _marker: PhantomData<(ConstraintF, AC, ACG, H, HG, HG)>,
}

impl<ConstraintF, AC, ACG, H, HG> ConstraintSynthesizer<ConstraintF>
    for TreeMembershipProver<ConstraintF, AC, ACG, H, HG>
where
    ConstraintF: PrimeField,
    AC: CommitmentScheme,
    AC::Output: ToConstraintField<ConstraintF>,
    ACG: CommitmentGadget<AC, ConstraintF>,
    H: TwoToOneCRH,
    H::Output: ToConstraintField<ConstraintF>,
    HG: TwoToOneCRHGadget<H, ConstraintF>,
{
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<ConstraintF>,
    ) -> Result<(), SynthesisError> {
        // Witness the public variables. In ALL zeronym proofs, it's the commitment to the
        // attributes and the merkle root
        let attrs_com_var =
            ACG::OutputVar::new_input(ns!(cs, "attrs com var"), || Ok(self.attrs_com))?;
        let root_var = HG::OutputVar::new_input(ns!(cs, "root var"), || Ok(self.root))?;

        // Now we do the tree membership proof. Input the two-to-one params
        let crh_param_var =
            HG::ParametersVar::new_constant(ns!(cs, "two_to_one param"), &self.crh_param)?;
        // This is a placeholder value. We don't actually use leaf hashes
        let leaf_param_var = UnitVar::default();

        // If there is no auth path, make one of the appropriate length
        let auth_path = match self.auth_path {
            Some(p) => p,
            None => {
                let default_com_bytes = to_bytes!(AC::Output::default()).unwrap();
                SparseMerkleTreePath::<ComTreeConfig<H>> {
                    leaf_hashes: (default_com_bytes.clone(), default_com_bytes),
                    inner_hashes: vec![
                        (H::Output::default(), H::Output::default());
                        self.height
                            .checked_sub(2)
                            .expect("tree height cannot be < 2")
                            as usize
                    ],
                }
            }
        };

        let path_var = SparseMerkleTreePathVar::<_, IdentityCRHGadget, HG, _>::new_witness(
            ns!(cs, "auth path"),
            || Ok(auth_path),
            self.height,
        )?;

        path_var.check_membership(
            ns!(cs, "check_membership").cs(),
            &leaf_param_var,
            &crh_param_var,
            &root_var,
            &attrs_com_var,
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::{
        NameAndBirthYear, TestComScheme, TestComSchemeG, TestTreeH, TestTreeHG, MERKLE_CRH_PARAM,
    };

    use ark_bls12_381::Bls12_381 as E;

    /// Tests a predicate that returns true iff the given `NameAndBirthYear` is at least 21
    #[test]
    fn test_com_tree_proof() {
        let mut rng = ark_std::test_rng();
        let tree_height = 32;

        // Make a attribute to put in the tree
        let person = NameAndBirthYear::new(&mut rng, b"Andrew", 1992);
        let person_com = person.commit();

        // Generate the predicate circuit's CRS
        let pk = gen_tree_memb_crs::<
            _,
            E,
            NameAndBirthYear,
            TestComScheme,
            TestComSchemeG,
            TestTreeH,
            TestTreeHG,
        >(&mut rng, MERKLE_CRH_PARAM.clone(), tree_height)
        .unwrap();

        // Make a tree and "issue", i.e., put the person commitment in the tree at index 17
        let leaf_idx = 17;
        let mut tree =
            ComTree::<_, TestTreeH, TestComScheme>::empty(MERKLE_CRH_PARAM.clone(), tree_height);
        tree.insert(leaf_idx, &person_com);

        // The person can now prove membership in the tree
        let proof = tree
            .prove_membership(&mut rng, &pk, leaf_idx, person_com)
            .unwrap();

        let vk = pk.prepare_verifying_key();
        assert!(verify_tree_memb(&vk, &proof, &person_com, &tree.root()).unwrap());
    }
}