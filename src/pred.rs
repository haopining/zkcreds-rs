use crate::{
    attrs::{Attrs, AttrsVar},
    proof_data_structures::{PredProof, PredProvingKey, PredPublicInput, PredVerifyingKey},
};

use core::marker::PhantomData;

use ark_crypto_primitives::commitment::{constraints::CommitmentGadget, CommitmentScheme};
use ark_ec::PairingEngine;
use ark_ff::PrimeField;
use ark_r1cs_std::{alloc::AllocVar, boolean::Boolean, eq::EqGadget};
use ark_relations::{
    ns,
    r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError},
};
use ark_std::rand::Rng;

/// Describes any predicate that someone might want to prove over an `Attrs` object.
pub trait PredicateChecker<ConstraintF, A, AV, AC, ACG>: Sized
where
    ConstraintF: PrimeField,
    A: Attrs<AC>,
    AV: AttrsVar<ConstraintF, A, AC, ACG>,
    AC: CommitmentScheme,
    ACG: CommitmentGadget<AC, ConstraintF>,
{
    /// Returns whether or not the predicate was satisfied
    fn pred(
        self,
        cs: ConstraintSystemRef<ConstraintF>,
        attrs: &AV,
    ) -> Result<Boolean<ConstraintF>, SynthesisError>;

    /// This outputs the field elements corresponding to the public inputs of this predicate. This
    /// DOES NOT include `attrs`.
    fn public_inputs(&self) -> Vec<ConstraintF>;
}

pub fn gen_pred_crs<R, P, E, A, AV, AC, ACG, MC, MCG>(
    rng: &mut R,
    checker: P,
) -> Result<PredProvingKey<E, A, AV, AC, ACG, MC, MCG>, SynthesisError>
where
    R: Rng,
    P: PredicateChecker<E::Fr, A, AV, AC, ACG>,
    E: PairingEngine,
    A: Attrs<AC>,
    AV: AttrsVar<E::Fr, A, AC, ACG>,
    AC: CommitmentScheme,
    ACG: CommitmentGadget<AC, E::Fr>,
    MC: CommitmentScheme,
    MCG: CommitmentGadget<MC, E::Fr>,
{
    let prover: PredicateProver<_, _, _, _, _, _, _, MCG> = PredicateProver {
        checker,
        attrs: A::default(),
        merkle_root_com: MC::Output::default(),
        _marker: PhantomData,
    };
    let pk = ark_groth16::generate_random_parameters(prover, rng)?;
    Ok(PredProvingKey {
        pk,
        _marker: PhantomData,
    })
}

pub fn prove_pred<R, P, E, A, AV, AC, ACG, MC, MCG>(
    rng: &mut R,
    pk: &PredProvingKey<E, A, AV, AC, ACG, MC, MCG>,
    checker: P,
    attrs: A,
    merkle_root_com: MC::Output,
) -> Result<PredProof<E, A, AV, AC, ACG, MC, MCG>, SynthesisError>
where
    R: Rng,
    P: PredicateChecker<E::Fr, A, AV, AC, ACG>,
    E: PairingEngine,
    A: Attrs<AC>,
    AV: AttrsVar<E::Fr, A, AC, ACG>,
    AC: CommitmentScheme,
    ACG: CommitmentGadget<AC, E::Fr>,
    MC: CommitmentScheme,
    MCG: CommitmentGadget<MC, E::Fr>,
{
    let prover: PredicateProver<_, _, _, _, _, _, _, MCG> = PredicateProver {
        checker,
        attrs,
        merkle_root_com,
        _marker: PhantomData,
    };
    let proof = ark_groth16::create_random_proof(prover, &pk.pk, rng)?;
    Ok(PredProof {
        proof,
        _marker: PhantomData,
    })
}

pub fn prepare_pred_inputs<R, P, E, A, AV, AC, ACG, MC, MCG>(
    vk: &PredVerifyingKey<E, A, AV, AC, ACG, MC, MCG>,
    checker: &P,
) -> Result<PredPublicInput<E, A, AV, AC, ACG, MC, MCG>, SynthesisError>
where
    R: Rng,
    P: PredicateChecker<E::Fr, A, AV, AC, ACG>,
    E: PairingEngine,
    A: Attrs<AC>,
    AV: AttrsVar<E::Fr, A, AC, ACG>,
    AC: CommitmentScheme,
    ACG: CommitmentGadget<AC, E::Fr>,
    MC: CommitmentScheme,
    MCG: CommitmentGadget<MC, E::Fr>,
{
    let pinput = ark_groth16::prepare_inputs(&vk.pvk, &checker.public_inputs())?;
    Ok(PredPublicInput {
        pinput,
        _marker: PhantomData,
    })
}

/// Internal object for proving predicates. This needs to implement `ConstraintSynthesizer` in
/// order to pass to the Groth16 proving functions. `AC` is the attribute commitment scheme, `MC`
/// is the merkle root commitment scheme.
pub(crate) struct PredicateProver<ConstraintF, P, A, AV, AC, ACG, MC, MCG>
where
    ConstraintF: PrimeField,
    P: PredicateChecker<ConstraintF, A, AV, AC, ACG>,
    A: Attrs<AC>,
    AV: AttrsVar<ConstraintF, A, AC, ACG>,
    AC: CommitmentScheme,
    ACG: CommitmentGadget<AC, ConstraintF>,
    MC: CommitmentScheme,
    MCG: CommitmentGadget<MC, ConstraintF>,
{
    checker: P,
    attrs: A,
    merkle_root_com: MC::Output,
    _marker: PhantomData<(ConstraintF, AV, AC, ACG, MCG)>,
}

impl<ConstraintF, P, A, AV, AC, ACG, MC, MCG> ConstraintSynthesizer<ConstraintF>
    for PredicateProver<ConstraintF, P, A, AV, AC, ACG, MC, MCG>
where
    ConstraintF: PrimeField,
    P: PredicateChecker<ConstraintF, A, AV, AC, ACG>,
    A: Attrs<AC>,
    AV: AttrsVar<ConstraintF, A, AC, ACG>,
    AC: CommitmentScheme,
    ACG: CommitmentGadget<AC, ConstraintF>,
    MC: CommitmentScheme,
    MCG: CommitmentGadget<MC, ConstraintF>,
{
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<ConstraintF>,
    ) -> Result<(), SynthesisError> {
        // Witness the public variables. In ALL zeronym proofs, it's the commitment to the
        // attributes and the merkle root
        let attrs_com_var =
            ACG::OutputVar::new_input(ns!(cs, "attrs com var"), || Ok(self.attrs.commit()))?;
        let _root_com_var =
            MCG::OutputVar::new_input(ns!(cs, "root com var"), || Ok(self.merkle_root_com))?;

        // Check that the attrs commitment is consistent
        let attrs_var = AV::new_witness(ns!(cs, "attrs var"), || Ok(&self.attrs))?;
        attrs_com_var.enforce_equal(&attrs_var.commit()?)?;

        // Finally assert the predicate is true
        let success = self.checker.pred(cs, &attrs_var)?;
        success.enforce_equal(&Boolean::TRUE)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::NameAndBirthYear;

    use ark_bls12_381::{Bls12_381 as E, Fr};

    // Make NameAndBirthYear a predicate prover

    #[test]
    fn it_works() {}
}