fn main(
    balance: Field,        // private witness
    bid_amount: pub Field, // public input
    price_ceiling: Field   // private witness
) {
    // Constraint 1: User has sufficient balance
    assert(balance >= bid_amount);
    
    // Constraint 2: Bid doesn't exceed the user's private limit
    assert(bid_amount <= price_ceiling);
    
    // Constraint 3: Bid must be positive
    assert(bid_amount > 0);
}
