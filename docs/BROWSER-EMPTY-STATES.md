# Browser empty-state ownership

The top-level browser painter previously drew an empty/loading message after
the dual-pane and Columns painters had already drawn their own. When the
active pane was empty, that global message could overlap files in the other
pane. Columns could show duplicate messages.

Each surface now owns its empty body. The single-browser fallback excludes
dual pane, Columns and SpaceMap. Dual pane and Columns keep their existing
clipping and placement; SpaceMap retains its specialized state.

The shared text resolver distinguishes a listing error, a pending listing,
an active filter with no results and an empty folder. Explicit errors take
priority over loading text. Short labels fit narrow panes. This does not
change navigation, selection, ordering, scrolling, listing workers or actions.

Three Painter regression tests cover the single-browser state matrix,
either empty dual pane with a populated sibling at minimum/normal sizes,
and scoped Columns states with populated neighboring columns. Private
framebuffer scenes cover the actual window and unchanged fixture payloads.
They are handler/render evidence; native OS input and resource/startup
acceptance remain separate.
