// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

// Derived from Chromium's accessibility abstraction.
// Copyright 2018 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE.chromium file.

use std::rc::{Rc, Weak};

use accesskit_consumer::{Node, NodeId, Tree};
use objc2::{ClassType, DeclaredClass, msg_send_id, rc::Id};
use objc2_foundation::NSArray;

use crate::{context::Context, filters::filter};

use super::{PlatformNode, PlatformNodeIvars};

impl PlatformNode {
    pub(crate) fn new(context: Weak<Context>, node_id: NodeId) -> Id<Self> {
        let this = Self::alloc().set_ivars(PlatformNodeIvars { context, node_id });

        unsafe { msg_send_id![super(this), init] }
    }

    pub(super) fn resolve_with_context<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&Node, &Tree, &Rc<Context>) -> T,
    {
        let context = self.ivars().context.upgrade()?;
        let tree = context.tree.borrow();
        let state = tree.state();
        let node = state.node_by_id(self.ivars().node_id)?;
        Some(f(&node, &tree, &context))
    }

    pub(super) fn resolve<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&Node) -> T,
    {
        self.resolve_with_context(|node, _, _| f(node))
    }

    pub(super) fn children_internal(&self) -> Option<Id<NSArray<PlatformNode>>> {
        self.resolve_with_context(|node, _, context| {
            let platform_nodes = node
                .filtered_children(filter)
                .map(|child| context.get_or_create_platform_node(child.id()))
                .collect::<Vec<Id<PlatformNode>>>();
            NSArray::from_vec(platform_nodes)
        })
    }
}
