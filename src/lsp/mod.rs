//! Language server lifecycle management and the three supported requests:
//! definition, references, and hover. Must be fully async and never block
//! the render loop; missing or slow servers degrade silently.
