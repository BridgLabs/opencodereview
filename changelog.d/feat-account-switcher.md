### Changed

- Switch the account new reviews run on from the model picker in the top bar. It lists each
  provider's accounts with the usage left, resumes a paused account when you pick it, and shows
  why an account at its limit can't be picked. The `+` button signs in a new Claude or ChatGPT
  account right there.
- Replace "Move up" and "Move down" in the account menu with "Use for new reviews".
- When an account hits its usage limit, reviews move to the next account of the same provider
  before trying another provider. Before, they followed one order across providers, so a ChatGPT
  account listed between two Claude accounts was used before the second Claude account.
