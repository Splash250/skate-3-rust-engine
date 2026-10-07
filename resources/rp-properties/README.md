# Borough Properties

The Boardwalk Borough property app demonstrates persistent, account-bound apartment leases. Players can rent the predefined studio using the economy service, enter its private instance, invite an online account, and return to their previous world. The map provides the studio interior and the resource assigns a bounded instance from 2000–2063.

The property service owns lease and invitation authorization. A rent request reserves its operation before charging, then reconciles the same idempotent wallet operation after a lost response or resource restart. Client messages contain named actions and player IDs only; price, account identity, instance and spawn location are server controlled.

Open **Properties** from the Phone app list or the player interface menu. Operators can set `max_guests` live; `studio_price` applies after resource restart. The database tables are managed through the resource migration service.
