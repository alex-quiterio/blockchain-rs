//! The values of proof of stake's randomness: a validator's commitment, the reveals that open
//! it, and the beacon they mix into. Value objects, each checked when it's made, and each its
//! own kind; the commit-reveal service that works them out is `randao`.

use crate::domain::crypto::sha256_type;

sha256_type!(
    /// A link of a validator's reveal chain, shown in a block it forges. Hashing it once gives the
    /// link shown before it, so anyone can check it, and nobody can show the next one early.
    Reveal
);

sha256_type!(
    /// The end of a validator's reveal chain, published in their first stake.
    Commitment
);

sha256_type!(
    /// The randomness that picks who forges next, which only validators' reveals move.
    Beacon
);
