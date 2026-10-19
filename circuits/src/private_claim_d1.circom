pragma circom 2.1.0;

include "private_claim.circom";

component main {public [root, commitment, lower, expiry]} = PrivateClaim(1, 20);
