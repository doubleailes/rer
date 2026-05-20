"""Package-orderer plugin SDK for ``pyrer``.

A *package orderer* decides, per family, which version the solver should
prefer. ``pyrer``'s default is rez's ``SortedOrder(descending=True)`` —
highest version first, using rez's native alphanumeric-token comparison.

To override that, subclass :class:`PackageOrderer`, implement
:meth:`PackageOrderer.order`, register the class with
:func:`register_orderer`, and select it on
``pyrer.solve(..., package_orderer="<name>")``::

    import pyrer

    class Pep440Orderer(pyrer.PackageOrderer):
        name = "pep440"

        def order(self, family, versions):
            # sort `versions` however you like, most-preferred first
            return sorted(versions, key=_pep440_key, reverse=True)

    pyrer.register_orderer(Pep440Orderer)
    result = pyrer.solve(requests, packages, package_orderer="pep440")

This mirrors rez's own orderer model — an SDK base class plus an explicit
registry — without rez's heavyweight plugin-manager discovery.
"""
from __future__ import annotations

from typing import Dict, List, Union

__all__ = ["PackageOrderer", "register_orderer"]


class PackageOrderer:
    """Base class for a ``pyrer`` package-orderer plugin.

    Subclass it, set the class attribute :attr:`name`, and implement
    :meth:`order`. Register the subclass (or an instance) with
    :func:`register_orderer`, then select it by name on
    ``pyrer.solve(..., package_orderer="<name>")``.
    """

    #: Registry key. A subclass **must** set this to a non-empty string.
    name: str = ""

    def order(self, family: str, versions: List[str]) -> List[str]:
        """Return ``versions`` reordered, most-preferred-first.

        ``family`` is the package family name; ``versions`` is every
        candidate version string the solver currently has for that family.

        The return value should be a permutation of ``versions``. ``pyrer``
        is defensive about a misbehaving orderer: a version omitted from the
        result sinks to the bottom (least preferred); a version in the
        result that was not in ``versions`` is ignored. The orderer is a
        *preference* function — it never changes whether a solve succeeds,
        only which solution is found first.

        Raising from this method propagates as a ``solve()`` result with
        ``status == "error"`` — no exception escapes ``pyrer.solve``.
        """
        raise NotImplementedError(
            f"{type(self).__name__} must implement order()"
        )


# Registry: orderer name -> PackageOrderer instance.
_orderers: Dict[str, "PackageOrderer"] = {}


def register_orderer(orderer: Union["PackageOrderer", type]) -> None:
    """Register a :class:`PackageOrderer` so it can be selected by name.

    Accepts either an instance or a :class:`PackageOrderer` *subclass*
    (instantiated with no arguments). The orderer's :attr:`~PackageOrderer.name`
    is the registry key; registering a second orderer under the same name
    replaces the first.

    Raises:
        TypeError: if `orderer` is not a `PackageOrderer` subclass/instance.
        ValueError: if the orderer's `name` is empty.
    """
    inst = orderer() if isinstance(orderer, type) else orderer
    if not isinstance(inst, PackageOrderer):
        raise TypeError(
            "register_orderer expects a PackageOrderer subclass or instance"
        )
    if not getattr(inst, "name", ""):
        raise ValueError(
            "a PackageOrderer must set a non-empty class attribute `name`"
        )
    _orderers[inst.name] = inst
