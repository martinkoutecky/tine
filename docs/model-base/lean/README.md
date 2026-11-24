# Listed images and patches

## Prior decisions

This package follows D-20261124-CWM. It provides the shared vocabulary for composing the conceptual and storage models.

A listed path is the exact graph-relative spelling. A listed entry contains a regular file's bytes, and a missing key has no entry. A finite image is an extensional map of the entries in scope. A patch has at most one effect per path, including its before and after entries, and a finite list of origin-to-destination bindings. A destination can have several origins. A binding can point to an unchanged entry, for example when a merge leaves its receiver's bytes unchanged.

Replay overlays the after entries. An absent after entry deletes the key. Each effect takes one map update, logarithmic in the current image size apart from path comparison. Checking the recorded before entries, resolving physical path aliases and scheduling publication belong to the caller.
