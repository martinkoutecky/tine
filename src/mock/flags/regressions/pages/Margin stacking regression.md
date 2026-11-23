- An opening block without comments, so the commented paragraph is not first.
	- A child of it.
- The proof bounds the error by the gradient norm, then sums the bounds over all rounds, and the sum telescopes because each step only shrinks the potential; the constant depends on the smoothness of the loss.
  tine.author:: claude
	- Which error, the training or the test error?
	  tine.quote:: bounds the error
	- Name the norm.
	  tine.quote:: gradient norm
	- Over all rounds or only the first T?
	  tine.quote:: over all rounds
		- All rounds; T is the horizon.
		  tine.author:: claude
	- Does it really telescope here?
	  tine.quote:: the sum telescopes
	- Shrinks by how much per step?
	  tine.quote:: only shrinks the potential
	- Smoothness constant L, say so.
	  tine.quote:: smoothness of the loss
- A closing block after the commented paragraph.
