### Security

- Never quote a private or internal repository linked from a pull request, or used as its
  submodule, in a review it could leak from. Linked code is accepted when it is public, or when
  the reviewed repository is private or internal and has the same owner; otherwise the review
  stops and says which link to remove. Before, any repository your GitHub session could read was
  frozen as evidence, so an outside contributor could link your private code into a public review.
- Accept `/ocr review` only from pull request authors who are an owner, member or collaborator of
  the repository. Before, any author could start a review on your AI accounts, even with
  automatic reviews limited to assigned pull requests.
