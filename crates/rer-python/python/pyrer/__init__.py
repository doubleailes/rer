"""pyrer — rer ("Rez En Rust"), a rez-compatible package resolver.

``pyrer`` is a mixed Rust+Python package: the compiled solver core lives in
the :mod:`pyrer._native` extension; this module re-exports it and adds the
package-orderer plugin SDK (:class:`~pyrer.orderer.PackageOrderer`,
:func:`~pyrer.orderer.register_orderer`).

The public surface is everything in ``__all__``. Do not import
``pyrer._native`` directly — it is an implementation detail.
"""
from __future__ import annotations

from pyrer._native import (
    PackageData,
    ResolvedVariant,
    SolveResult,
    parse_static_package_py,
    parse_static_packages_py,
)
from pyrer._native import solve as _native_solve
from pyrer.orderer import PackageOrderer, _orderers, register_orderer

__all__ = [
    "solve",
    "PackageData",
    "ResolvedVariant",
    "SolveResult",
    "parse_static_package_py",
    "parse_static_packages_py",
    "PackageOrderer",
    "register_orderer",
]

try:
    from importlib.metadata import version as _pkg_version

    __version__ = _pkg_version("pyrer")
except Exception:  # pragma: no cover - version metadata is best-effort
    __version__ = "unknown"


def solve(
    package_requests,
    packages=None,
    /,
    *,
    load_family=None,
    variant_select_mode="version_priority",
    package_orderer=None,
):
    """Resolve ``package_requests`` against a package repository.

    Args:
        package_requests: rez-style requirement strings, e.g.
            ``["python-3", "maya-2024"]``.
        packages: a ``list[PackageData]`` (the eager repository), or
            ``None`` when discovery is fully driven by ``load_family``.
        load_family: optional ``Callable`` invoked on demand the first
            time the solver needs a family it has not seen. See the rez
            integration docs for the callback contract.
        variant_select_mode: ``"version_priority"`` (rez's default) or
            ``"intersection_priority"``.
        package_orderer: overrides the per-family version preference. Pass
            the registered **name** of a :class:`PackageOrderer` (a
            ``str``), a :class:`PackageOrderer` **instance** directly, or
            ``None`` for the default (highest version first). Register an
            orderer with :func:`register_orderer` before selecting it by
            name.

    Returns:
        A :class:`SolveResult`. Failures and bad input are reported via
        ``result.status``, never as a Python exception.

    Raises:
        ValueError: if ``package_orderer`` is a name with no registered
            orderer.
        TypeError: if ``package_orderer`` is not a str / PackageOrderer /
            None, or ``packages`` is the wrong type.
    """
    order_fn = None
    if package_orderer is not None:
        if isinstance(package_orderer, str):
            inst = _orderers.get(package_orderer)
            if inst is None:
                raise ValueError(
                    f"no package orderer registered as {package_orderer!r} — "
                    f"register one with pyrer.register_orderer()"
                )
        elif isinstance(package_orderer, PackageOrderer):
            inst = package_orderer
        else:
            raise TypeError(
                "package_orderer must be a str, a PackageOrderer, or None"
            )
        # Bound method (family, versions) -> reordered versions; this is
        # the plain callable the Rust core consumes.
        order_fn = inst.order

    return _native_solve(
        package_requests,
        packages,
        load_family=load_family,
        variant_select_mode=variant_select_mode,
        package_order=order_fn,
    )
